use crate::config::EnvPaths;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct MariaDbService {
    child: Option<Child>,
    pub port: u16,
}

impl MariaDbService {
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
            // Runtime directory search
            paths.runtime_dir.join("linux-x86_64/mariadb/bin/mariadbd"),
            paths
                .runtime_dir
                .join("windows-x86_64/mariadb/bin/mysqld.exe"),
            paths
                .runtime_dir
                .join("windows-x86_64/mariadb/bin/mariadbd.exe"),
            paths.runtime_dir.join("macos-arm64/mariadb/bin/mariadbd"),
            paths.runtime_dir.join("macos-x86_64/mariadb/bin/mariadbd"),
            paths.runtime_dir.join("mariadb/bin/mariadbd"),
            paths.runtime_dir.join("mariadb/bin/mysqld"),
        ];

        for path in &candidates {
            if path.exists() {
                return Ok(path.clone());
            }
        }

        // Check any matching runtime/* subdirectory
        if let Ok(entries) = std::fs::read_dir(&paths.runtime_dir) {
            for entry in entries.flatten() {
                let m_bin = entry.path().join("mariadb/bin/mariadbd");
                if m_bin.exists() {
                    return Ok(m_bin);
                }
                let w_bin = entry.path().join("mariadb/bin/mysqld.exe");
                if w_bin.exists() {
                    return Ok(w_bin);
                }
            }
        }

        // Dev fallback: PATH lookup
        for name in &["mariadbd", "mysqld"] {
            if let Ok(path) = which::which(name) {
                return Ok(path);
            }
        }

        bail!("MariaDB binary not found in runtime directory or system PATH")
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
        let datadir = paths.data_dir.join("mariadb");
        let error_log = paths.logs_dir.join("mariadb_error.log");

        // First-time database init if mysql system database directory doesn't exist
        if !datadir.join("mysql").exists() {
            if let Ok(entries) = std::fs::read_dir(&datadir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let _ = std::fs::remove_file(path);
                    } else if path.is_dir() {
                        let _ = std::fs::remove_dir_all(path);
                    }
                }
            }
            Self::initialize_db(paths, &datadir)?;
        }

        let datadir_str = datadir.to_string_lossy().replace('\\', "/");
        let error_log_str = error_log.to_string_lossy().replace('\\', "/");
        let socket_str = paths
            .tmp_dir
            .join("mysql.sock")
            .to_string_lossy()
            .replace('\\', "/");
        let pid_str = paths
            .tmp_dir
            .join("mariadb.pid")
            .to_string_lossy()
            .replace('\\', "/");
        let tmp_str = paths.tmp_dir.to_string_lossy().replace('\\', "/");

        let mut cmd = Command::new(&bin);
        // --no-defaults MUST be the first argument to prevent loading host system /etc/my.cnf.d/*.cnf
        cmd.arg("--no-defaults")
            .arg(format!("--datadir={}", datadir_str))
            .arg(format!("--port={}", self.port))
            .arg("--bind-address=127.0.0.1")
            .arg(format!("--log-error={}", error_log_str))
            .arg(format!("--socket={}", socket_str))
            .arg(format!("--pid-file={}", pid_str))
            .arg(format!("--tmpdir={}", tmp_str))
            .arg("--default-storage-engine=InnoDB")
            .arg("--skip-networking=0")
            .arg("--innodb-flush-log-at-trx-commit=2")
            .arg("--innodb-doublewrite=0")
            .arg("--innodb-buffer-pool-size=256M")
            .arg("--innodb-io-capacity=2000")
            .arg("--innodb-io-capacity-max=4000")
            .arg("--innodb-read-io-threads=4")
            .arg("--innodb-write-io-threads=4")
            .arg("--skip-log-bin")
            .arg("--max-allowed-packet=64M")
            .arg("--wait-timeout=600")
            .arg("--interactive-timeout=600")
            .arg("--net-read-timeout=600")
            .arg("--net-write-timeout=600")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Set --basedir if bundled MariaDB
        if let Some(base) = bin.parent().and_then(|p| p.parent()) {
            if base.join("share/english").exists()
                || base.join("share/charsets").exists()
                || base.join("share").exists()
            {
                let base_str = base.to_string_lossy().replace('\\', "/");
                cmd.arg(format!("--basedir={}", base_str));
            }
        }

        let init_sql = paths.tmp_dir.join("init_prestashop.sql");
        let _ = std::fs::write(
            &init_sql,
            "CREATE DATABASE IF NOT EXISTS `prestashop` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n",
        );
        let init_sql_str = init_sql.to_string_lossy().replace('\\', "/");
        cmd.arg(format!("--init-file={}", init_sql_str));

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn MariaDB process {:?}", bin))?;

        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Ok(Some(status)) = child.try_wait() {
            let log_tail = std::fs::read_to_string(&error_log).unwrap_or_default();
            let recent_lines = log_tail
                .lines()
                .rev()
                .take(15)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "MariaDB failed to start (status: {}).\nLast error log entries:\n{}",
                status,
                recent_lines
            );
        }

        self.child = Some(child);
        Ok(())
    }

    fn initialize_db(paths: &EnvPaths, datadir: &Path) -> Result<()> {
        // Search for install-db binary
        let mut install_bin = None;
        for sub in &[
            "windows-x86_64",
            "linux-x86_64",
            "macos-arm64",
            "macos-x86_64",
            "",
        ] {
            for candidate in &[
                "mariadb/bin/mariadb-install-db.exe",
                "mariadb/bin/mysql_install_db.exe",
                "mariadb/bin/mariadb-install-db",
                "mariadb/scripts/mysql_install_db",
            ] {
                let p = paths.runtime_dir.join(sub).join(candidate);
                if p.exists() {
                    install_bin = Some(p);
                    break;
                }
            }
            if install_bin.is_some() {
                break;
            }
        }

        if install_bin.is_none() {
            if let Ok(entries) = std::fs::read_dir(&paths.runtime_dir) {
                for entry in entries.flatten() {
                    for candidate in &[
                        "mariadb/bin/mariadb-install-db.exe",
                        "mariadb/bin/mysql_install_db.exe",
                        "mariadb/bin/mariadb-install-db",
                        "mariadb/scripts/mysql_install_db",
                    ] {
                        let bin = entry.path().join(candidate);
                        if bin.exists() {
                            install_bin = Some(bin);
                            break;
                        }
                    }
                    if install_bin.is_some() {
                        break;
                    }
                }
            }
        }

        if install_bin.is_none() {
            for name in &[
                "mariadb-install-db.exe",
                "mysql_install_db.exe",
                "mariadb-install-db",
                "mysql_install_db",
            ] {
                if let Ok(path) = which::which(name) {
                    install_bin = Some(path);
                    break;
                }
            }
        }

        let installer = match install_bin {
            Some(i) => i,
            None => bail!("MariaDB database installer binary (mariadb-install-db / mysql_install_db) not found"),
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = installer.metadata() {
                let mut perms = metadata.permissions();
                let mode = perms.mode();
                if mode & 0o111 != 0o111 {
                    perms.set_mode(mode | 0o755);
                    let _ = std::fs::set_permissions(&installer, perms);
                }
            }
        }

        let mut init_cmd = Command::new(&installer);

        // Capture installation output to logs/mariadb_install.log
        let install_log_path = paths.logs_dir.join("mariadb_install.log");
        if let Ok(log_file) = std::fs::File::create(&install_log_path) {
            if let Ok(err_file) = log_file.try_clone() {
                init_cmd.stdout(Stdio::from(log_file));
                init_cmd.stderr(Stdio::from(err_file));
            }
        }

        #[cfg(windows)]
        {
            let datadir_str = datadir.to_string_lossy().replace('\\', "/");
            init_cmd.arg(format!("--datadir={}", datadir_str));
            if let Some(base) = installer.parent().and_then(|p| p.parent()) {
                init_cmd.current_dir(base);
            }
            use std::os::windows::process::CommandExt;
            init_cmd.creation_flags(0x08000000);
        }

        #[cfg(not(windows))]
        {
            init_cmd
                .arg("--no-defaults")
                .arg(format!("--datadir={}", datadir.to_string_lossy()))
                .arg("--auth-root-authentication-method=normal")
                .arg("--skip-test-db")
                .arg("--force");

            if let Some(base) = installer.parent().and_then(|p| p.parent()) {
                init_cmd.current_dir(base);
                if base.join("share/english").exists() || base.join("share/charsets").exists() {
                    init_cmd.arg(format!("--basedir={}", base.to_string_lossy()));
                }
            }
        }

        let status = init_cmd
            .status()
            .with_context(|| format!("Failed to spawn DB installer {:?}", installer))?;
        drop(init_cmd);
        if !status.success() {
            let log_snippet = std::fs::read_to_string(&install_log_path).unwrap_or_default();
            bail!(
                "MariaDB database initialization failed with status: {}.\nInstaller log:\n{}",
                status,
                log_snippet.trim()
            );
        }
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

            // Wait up to 3 seconds for clean shutdown
            for _ in 0..30 {
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

impl Drop for MariaDbService {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
