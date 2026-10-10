use crate::config::EnvPaths;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

pub struct PhpService {
    child: Option<Child>,
    pub port: u16,
}

impl PhpService {
    pub fn new(port: u16) -> Self {
        Self { child: None, port }
    }

    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(None) => true,
                _ => {
                    self.child = None;
                    false
                }
            }
        } else {
            false
        }
    }

    pub fn find_binary(paths: &EnvPaths) -> Result<PathBuf> {
        let candidates = [
            paths.runtime_dir.join("linux-x86_64/php/php-fpm"),
            paths.runtime_dir.join("linux-x86_64/php/php-cgi"),
            paths.runtime_dir.join("windows-x86_64/php/php-cgi.exe"),
            paths.runtime_dir.join("macos-arm64/php/php-fpm"),
            paths.runtime_dir.join("macos-arm64/php/php-cgi"),
            paths.runtime_dir.join("macos-x86_64/php/php-fpm"),
            paths.runtime_dir.join("macos-x86_64/php/php-cgi"),
            paths.runtime_dir.join("php/php-fpm"),
            paths.runtime_dir.join("php/php-cgi"),
            paths.runtime_dir.join("php/php-cgi.exe"),
        ];

        for path in &candidates {
            if path.exists() {
                return Ok(path.clone());
            }
        }

        if let Ok(entries) = std::fs::read_dir(&paths.runtime_dir) {
            for entry in entries.flatten() {
                for sub in &["php/php-fpm", "php/php-cgi", "php/php-cgi.exe"] {
                    let bin = entry.path().join(sub);
                    if bin.exists() {
                        return Ok(bin);
                    }
                }
            }
        }

        for name in &["php-fpm", "php-cgi", "php-cgi.exe"] {
            if let Ok(path) = which::which(name) {
                return Ok(path);
            }
        }

        bail!("PHP binary (php-fpm or php-cgi) not found in runtime directory or system PATH")
    }

    pub fn start(&mut self, paths: &EnvPaths) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }

        let bin = Self::find_binary(paths)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = bin.metadata() {
                let mut perms = metadata.permissions();
                let mode = perms.mode();
                if mode & 0o111 != 0o111 {
                    perms.set_mode(mode | 0o755);
                    let _ = std::fs::set_permissions(&bin, perms);
                }
            }
        }
        let php_ini = paths.generate_php_ini()?;

        let is_fpm = bin
            .file_name()
            .map(|n| n.to_string_lossy().contains("fpm"))
            .unwrap_or(false);

        let mut cmd = Command::new(&bin);

        if is_fpm {
            let fpm_conf = paths.tmp_dir.join("php-fpm.conf");
            let fpm_content = format!(
                "[global]\nerror_log = \"{}\"\ndaemonize = no\n\n[www]\nlisten = 127.0.0.1:{}\npm = static\npm.max_children = 4\npm.max_requests = 1000\nrequest_terminate_timeout = 600s\ncatch_workers_output = yes\nphp_admin_value[error_log] = \"{}\"\nphp_admin_flag[log_errors] = on\n",
                paths.logs_dir.join("php_fpm_error.log").to_string_lossy(),
                self.port,
                paths.logs_dir.join("php_errors.log").to_string_lossy(),
            );
            std::fs::write(&fpm_conf, fpm_content)?;
            cmd.arg("-F")
                .arg("-y")
                .arg(fpm_conf.to_string_lossy().to_string())
                .arg("-c")
                .arg(php_ini.to_string_lossy().to_string());
        } else {
            cmd.arg("-b")
                .arg(format!("127.0.0.1:{}", self.port))
                .arg("-c")
                .arg(php_ini.to_string_lossy().to_string())
                .env("PHP_FCGI_CHILDREN", "4")
                .env("PHP_FCGI_MAX_REQUESTS", "1000");
        }

        cmd.env("PHP_INI_SCAN_DIR", "")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let openssl_cnf = paths.config_dir.join("openssl.cnf");
        if openssl_cnf.exists() {
            cmd.env("OPENSSL_CONF", &openssl_cnf);
        } else if let Some(bin_dir) = bin.parent() {
            let bundled_cnf = bin_dir.join("extras/ssl/openssl.cnf");
            if bundled_cnf.exists() {
                cmd.env("OPENSSL_CONF", &bundled_cnf);
            }
        }

        let cacert_pem = paths.config_dir.join("cacert.pem");
        if cacert_pem.exists() {
            cmd.env("SSL_CERT_FILE", &cacert_pem);
        }

        if let Some(bin_dir) = bin.parent() {
            let mut env_paths =
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                    .collect::<Vec<_>>();
            if !env_paths.iter().any(|p| p == bin_dir) {
                env_paths.insert(0, bin_dir.to_path_buf());
            }
            if let Ok(new_path) = std::env::join_paths(env_paths) {
                cmd.env("PATH", new_path);
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn PHP process {:?}", bin))?;

        let _ = std::fs::write(paths.tmp_dir.join("php.pid"), child.id().to_string());

        self.child = Some(child);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            {
                unsafe {
                    libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
                }
            }
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &child.id().to_string()])
                    .status();
            }

            for _ in 0..20 {
                if let Ok(Some(_)) = child.try_wait() {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

impl Drop for PhpService {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
