use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub web_port: u16,
    pub php_port: u16,
    pub db_port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            web_port: 8080,
            php_port: 9000,
            db_port: 3306,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnvPaths {
    pub root_dir: PathBuf,
    pub app_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub config_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub data_dir: PathBuf,
    pub tmp_dir: PathBuf,
}

impl EnvPaths {
    pub fn resolve() -> Result<Self> {
        let mut candidates = Vec::new();
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(parent) = exe_path.parent() {
                candidates.push(parent.to_path_buf());
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd);
        }

        let mut root_dir = None;
        for start in candidates {
            let mut curr = start;
            for _ in 0..5 {
                if curr.join("config").exists()
                    || curr.join("versions.env").exists()
                    || curr.join("prd.md").exists()
                {
                    root_dir = Some(curr);
                    break;
                }
                if let Some(parent) = curr.parent() {
                    curr = parent.to_path_buf();
                } else {
                    break;
                }
            }
            if root_dir.is_some() {
                break;
            }
        }

        let root_dir = root_dir.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let mut app_dir = root_dir.join("app");
        if !app_dir.exists() && root_dir.join("prestashop").exists() {
            app_dir = root_dir.join("prestashop");
        }

        let runtime_dir = root_dir.join("runtime");
        let config_dir = root_dir.join("config");
        let logs_dir = root_dir.join("logs");
        let data_dir = root_dir.join("data");
        let tmp_dir = root_dir.join("tmp");

        fs::create_dir_all(&logs_dir)?;
        fs::create_dir_all(tmp_dir.join("sessions"))?;
        fs::create_dir_all(tmp_dir.join("uploads"))?;
        fs::create_dir_all(tmp_dir.join("nginx_client_body"))?;
        fs::create_dir_all(tmp_dir.join("nginx_proxy"))?;
        fs::create_dir_all(tmp_dir.join("nginx_fastcgi"))?;
        fs::create_dir_all(tmp_dir.join("nginx_uwsgi"))?;
        fs::create_dir_all(tmp_dir.join("nginx_scgi"))?;
        fs::create_dir_all(root_dir.join("temp"))?;
        fs::create_dir_all(data_dir.join("mariadb"))?;
        if app_dir.exists() {
            let _ = Self::setup_cache_directory(&app_dir);
            let _ = fs::create_dir_all(app_dir.join("var/logs"));
            let download_dir = app_dir.join("download");
            let _ = fs::create_dir_all(&download_dir);
            let download_index = download_dir.join("index.php");
            if !download_index.exists() {
                let _ = fs::write(&download_index, "<?php\n");
            }

            // Ensure PrestaShop .env file exists to prevent Symfony Dotenv PathException crashes
            let env_path = app_dir.join(".env");
            if !env_path.exists() {
                let env_dist = app_dir.join(".env.dist");
                if env_dist.exists() {
                    let _ = fs::copy(&env_dist, &env_path);
                } else {
                    let default_env = "# PrestaShop environment configuration\nPS_FF_FRONT_CONTAINER_V2=false\nPS_TRUSTED_PROXIES=\n";
                    let _ = fs::write(&env_path, default_env);
                }
            }
        }

        let paths = Self {
            root_dir,
            app_dir,
            runtime_dir,
            config_dir,
            logs_dir,
            data_dir,
            tmp_dir,
        };
        paths.ensure_runtime_permissions();

        Ok(paths)
    }

    /// Recursively ensures executable permissions on runtime binaries on Unix systems
    pub fn ensure_runtime_permissions(&self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fn make_exec_recursive(dir: &std::path::Path) {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            make_exec_recursive(&path);
                        } else if path.is_file() {
                            let should_exec = path
                                .parent()
                                .and_then(|p| p.file_name())
                                .map(|name| {
                                    let s = name.to_string_lossy();
                                    s == "bin" || s == "sbin" || s == "scripts" || s == "php"
                                })
                                .unwrap_or(false);

                            if should_exec {
                                if let Ok(metadata) = path.metadata() {
                                    let mut perms = metadata.permissions();
                                    let mode = perms.mode();
                                    if mode & 0o111 != 0o111 {
                                        perms.set_mode(mode | 0o755);
                                        let _ = std::fs::set_permissions(&path, perms);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if self.runtime_dir.exists() {
                make_exec_recursive(&self.runtime_dir);

                #[cfg(target_os = "macos")]
                {
                    let _ = std::process::Command::new("xattr")
                        .args([
                            "-r",
                            "-d",
                            "com.apple.quarantine",
                            &self.runtime_dir.to_string_lossy(),
                        ])
                        .status();
                }
            }
        }
    }

    /// Detect active PrestaShop state:
    /// Returns (is_setup_mode, detected_admin_folder_name)
    pub fn detect_prestashop_state(&self) -> (bool, Option<String>) {
        if !self.app_dir.exists() {
            return (true, None);
        }

        let install_dir = self.app_dir.join("install");
        let has_install_dir = install_dir.exists() && install_dir.is_dir();

        // Scan for admin directory (admin, admin_*, admin[0-9]*, etc.)
        let mut found_admin = None;
        if let Ok(entries) = fs::read_dir(&self.app_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name != "admin-api"
                            && name != "admin-dev"
                            && name.starts_with("admin")
                            && path.join("index.php").exists()
                        {
                            // Prefer customized/renamed admin directory (e.g. admin_xyz, admin982a1f)
                            if name != "admin" {
                                found_admin = Some(name.to_string());
                                break;
                            } else if found_admin.is_none() {
                                found_admin = Some(name.to_string());
                            }
                        }
                    }
                }
            }
        }

        let is_setup = has_install_dir || found_admin.is_none();
        (is_setup, found_admin)
    }

    /// Generates active php.ini from template
    pub fn generate_php_ini(&self) -> Result<PathBuf> {
        let template_path = self.config_dir.join("php.ini.template");
        let active_path = self.config_dir.join("php.ini");

        let content = if template_path.exists() {
            fs::read_to_string(&template_path)?
        } else {
            include_str!("../../config/php.ini.template").to_string()
        };

        let root_str = self.root_dir.to_string_lossy().replace('\\', "/");
        let mut active_content = content.replace("{{ROOT}}", &root_str);

        // Resolve PHP extensions directory if present (e.g. Windows precompiled PHP)
        let mut extensions_block = String::new();
        let mut ext_dir = None;
        for sub in &["windows-x86_64/php/ext", "php/ext"] {
            let candidate = self.runtime_dir.join(sub);
            if candidate.exists() {
                ext_dir = Some(candidate);
                break;
            }
        }
        if ext_dir.is_none() {
            if let Ok(entries) = std::fs::read_dir(&self.runtime_dir) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("php/ext");
                    if candidate.exists() {
                        ext_dir = Some(candidate);
                        break;
                    }
                }
            }
        }

        if let Some(ext_path) = ext_dir {
            let ext_str = ext_path.to_string_lossy().replace('\\', "/");
            extensions_block.push_str(&format!("extension_dir = \"{}\"\n", ext_str));

            let exts = [
                "curl",
                "fileinfo",
                "gd",
                "intl",
                "mbstring",
                "mysqli",
                "openssl",
                "pdo_mysql",
                "soap",
                "sockets",
                "sodium",
                "sqlite3",
                "exif",
                "zip",
            ];

            for ext in &exts {
                let dll_name = format!("php_{}.dll", ext);
                if ext_path.join(&dll_name).exists() {
                    extensions_block.push_str(&format!("extension={}\n", ext));
                }
            }

            if ext_path.join("php_opcache.dll").exists() {
                extensions_block.push_str("zend_extension=opcache\n");
            }
        }

        if active_content.contains("{{PHP_EXTENSIONS}}") {
            active_content = active_content.replace("{{PHP_EXTENSIONS}}", &extensions_block);
        } else if !extensions_block.is_empty() {
            active_content.push_str("\n;; Dynamic Extensions\n");
            active_content.push_str(&extensions_block);
        }

        fs::write(&active_path, active_content)
            .with_context(|| format!("Failed to write active php.ini to {:?}", active_path))?;

        Ok(active_path)
    }

    /// Generates active nginx.conf from template
    pub fn generate_nginx_conf(&self, web_port: u16, php_port: u16) -> Result<PathBuf> {
        let template_path = self.config_dir.join("nginx.conf.template");
        let active_path = self.config_dir.join("nginx.conf");

        let content = if template_path.exists() {
            fs::read_to_string(&template_path)?
        } else {
            include_str!("../../config/nginx.conf.template").to_string()
        };

        let root_str = self.root_dir.to_string_lossy().replace('\\', "/");
        let app_str = self.app_dir.to_string_lossy().replace('\\', "/");

        let active_content = content
            .replace("{{ROOT}}", &root_str)
            .replace("{{APP_DIR}}", &app_str)
            .replace("{{PORT}}", &web_port.to_string())
            .replace("{{PHP_PORT}}", &php_port.to_string());

        fs::write(&active_path, active_content)
            .with_context(|| format!("Failed to write active nginx.conf to {:?}", active_path))?;

        Ok(active_path)
    }

    /// Checks if a given path is located inside a cloud-synchronized folder (OneDrive, Dropbox, etc.)
    pub fn is_cloud_synced_path(path: &Path) -> bool {
        let path_str = path.to_string_lossy().to_lowercase();
        let normalized = path_str.replace('\\', "/");

        // Component-level check to avoid false positives (e.g. "clonedrive")
        for comp in path.components() {
            let comp_str = comp.as_os_str().to_string_lossy().to_lowercase();
            if comp_str == "onedrive"
                || comp_str.starts_with("onedrive ")
                || comp_str.starts_with("onedrive -")
                || comp_str == "dropbox"
                || comp_str.starts_with("dropbox ")
                || comp_str == "google drive"
                || comp_str == "googledrive"
                || comp_str == "iclouddrive"
                || comp_str == "icloud drive"
            {
                return true;
            }
        }

        // Substring check for normalized unix-like or cross-platform strings
        let patterns = [
            "/onedrive/",
            "/onedrive -",
            "/onedrive ",
            "/dropbox/",
            "/google drive/",
            "/googledrive/",
            "/iclouddrive/",
            "/icloud drive/",
        ];
        for pat in &patterns {
            if normalized.contains(pat) {
                return true;
            }
        }

        for env_key in &["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Ok(onedrive_root) = std::env::var(env_key) {
                if !onedrive_root.is_empty() {
                    let onedrive_norm = onedrive_root.to_lowercase().replace('\\', "/");
                    if normalized.starts_with(&onedrive_norm) {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Resolves the isolated cache target directory in Local AppData or temporary directory
    pub fn resolve_isolated_cache_target(app_dir: &Path) -> PathBuf {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        let normalized = app_dir.to_string_lossy().to_lowercase().replace('\\', "/");
        normalized.hash(&mut hasher);
        let hash_str = format!("{:016x}", hasher.finish());

        #[cfg(windows)]
        {
            let base = std::env::var("LOCALAPPDATA")
                .or_else(|_| std::env::var("APPDATA"))
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            base.join("PrestaShopPortable").join("cache").join(hash_str)
        }

        #[cfg(not(windows))]
        {
            std::env::temp_dir()
                .join("prestashop-portable")
                .join("cache")
                .join(hash_str)
        }
    }

    /// Sets up the PrestaShop var/cache directory.
    /// If app_dir is inside a cloud-synced folder (such as OneDrive on Windows),
    /// this automatically redirects var/cache to an isolated local directory via an NTFS Directory Junction,
    /// preventing sharing violations and atomic-rename lockups in Twig and Symfony.
    pub fn setup_cache_directory(app_dir: &Path) -> Result<PathBuf> {
        let var_dir = app_dir.join("var");
        let cache_dir = var_dir.join("cache");
        fs::create_dir_all(&var_dir)?;

        let is_cloud_synced = Self::is_cloud_synced_path(app_dir);

        if is_cloud_synced {
            let target = Self::resolve_isolated_cache_target(app_dir);
            fs::create_dir_all(&target)?;

            let is_symlink_or_junction = cache_dir
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);

            if is_symlink_or_junction {
                if cache_dir.exists() {
                    return Ok(target);
                }
                // Broken link: remove it so we can re-create
                let _ = fs::remove_file(&cache_dir).or_else(|_| fs::remove_dir(&cache_dir));
            }

            // If it exists as a regular directory, clear or move it
            if cache_dir.exists() {
                if fs::remove_dir_all(&cache_dir).is_err() {
                    let stale_name = format!(
                        "cache_old_{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    );
                    let _ = fs::rename(&cache_dir, var_dir.join(stale_name));
                }
            }

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let status = std::process::Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        &cache_dir.to_string_lossy(),
                        &target.to_string_lossy(),
                    ])
                    .creation_flags(0x08000000)
                    .status();

                if status.is_err() || !status.unwrap().success() {
                    if std::os::windows::fs::symlink_dir(&target, &cache_dir).is_err() {
                        let _ = fs::create_dir_all(&cache_dir);
                    }
                }
            }

            #[cfg(unix)]
            {
                let _ = std::os::unix::fs::symlink(&target, &cache_dir);
                if !cache_dir.exists() {
                    let _ = fs::create_dir_all(&cache_dir);
                }
            }

            return Ok(target);
        }

        fs::create_dir_all(&cache_dir)?;
        Ok(cache_dir)
    }

    /// Cleans the cache directory while preserving any active symlink or junction
    pub fn clean_cache_directory(app_dir: &Path) -> Result<()> {
        let cache_dir = app_dir.join("var/cache");
        if !cache_dir.exists() {
            return Self::setup_cache_directory(app_dir).map(|_| ());
        }

        let is_symlink_or_junction = cache_dir
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);

        if is_symlink_or_junction {
            if let Ok(entries) = fs::read_dir(&cache_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let _ = fs::remove_dir_all(&path);
                    } else {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        } else {
            let _ = fs::remove_dir_all(&cache_dir);
            let _ = Self::setup_cache_directory(app_dir);
        }

        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.web_port, 8080);
        assert_eq!(config.php_port, 9000);
        assert_eq!(config.db_port, 3306);
    }

    #[test]
    fn test_resolve_paths() {
        let paths = EnvPaths::resolve().expect("Should resolve env paths");
        assert!(paths.root_dir.exists());
        assert!(paths.logs_dir.exists());
        assert!(paths.data_dir.exists());
    }

    #[test]
    fn test_generate_configs() {
        let paths = EnvPaths::resolve().expect("Should resolve env paths");
        let php_ini = paths.generate_php_ini().expect("Should generate php.ini");
        assert!(php_ini.exists());

        let nginx_conf = paths
            .generate_nginx_conf(8080, 9000)
            .expect("Should generate nginx.conf");
        assert!(nginx_conf.exists());
    }

    #[test]
    fn test_detect_prestashop_state() {
        let temp_dir = std::env::temp_dir().join(format!("test_ps_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let mut paths = EnvPaths::resolve().expect("Should resolve env paths");
        paths.app_dir = temp_dir.clone();

        // 1. Empty app dir -> setup mode, no admin
        let (is_setup, admin) = paths.detect_prestashop_state();
        assert!(is_setup);
        assert_eq!(admin, None);

        // 2. Install folder + default admin -> setup mode
        fs::create_dir_all(temp_dir.join("install")).unwrap();
        fs::create_dir_all(temp_dir.join("admin")).unwrap();
        fs::write(temp_dir.join("admin/index.php"), "<?php").unwrap();
        let (is_setup, admin) = paths.detect_prestashop_state();
        assert!(is_setup);
        assert_eq!(admin, Some("admin".to_string()));

        // 3. Removed install folder + renamed admin982a1f -> ready mode, renamed admin preferred
        fs::remove_dir_all(temp_dir.join("install")).unwrap();
        fs::create_dir_all(temp_dir.join("admin982a1f")).unwrap();
        fs::write(temp_dir.join("admin982a1f/index.php"), "<?php").unwrap();
        // Also simulate admin-api to ensure it's not chosen
        fs::create_dir_all(temp_dir.join("admin-api")).unwrap();
        fs::write(temp_dir.join("admin-api/index.php"), "<?php").unwrap();

        let (is_setup, admin) = paths.detect_prestashop_state();
        assert!(!is_setup);
        assert_eq!(admin, Some("admin982a1f".to_string()));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_generate_php_ini_with_extensions() {
        let temp_dir = std::env::temp_dir().join(format!("test_ext_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let ext_dir = temp_dir.join("runtime/php/ext");
        fs::create_dir_all(&ext_dir).unwrap();
        fs::write(ext_dir.join("php_zip.dll"), "").unwrap();
        fs::write(ext_dir.join("php_curl.dll"), "").unwrap();

        let mut paths = EnvPaths::resolve().expect("Should resolve env paths");
        paths.runtime_dir = temp_dir.join("runtime");
        paths.config_dir = temp_dir.join("config");
        fs::create_dir_all(&paths.config_dir).unwrap();
        let php_ini = paths.generate_php_ini().expect("Should generate php.ini");
        let ini_content = fs::read_to_string(&php_ini).unwrap();
        assert!(ini_content.contains("extension=zip"));
        assert!(ini_content.contains("extension=curl"));
        assert!(ini_content.contains("extension_dir ="));

        let _ = fs::remove_dir_all(&temp_dir);
        // Regenerate clean php.ini
        let orig_paths = EnvPaths::resolve().expect("Should resolve env paths");
        let _ = orig_paths.generate_php_ini();
    }

    #[test]
    fn test_is_cloud_synced_path() {
        assert!(EnvPaths::is_cloud_synced_path(std::path::Path::new(r"C:\Users\John\OneDrive\Desktop\app")));
        assert!(EnvPaths::is_cloud_synced_path(std::path::Path::new(r"C:\Users\John\OneDrive - Org\Documents\app")));
        assert!(EnvPaths::is_cloud_synced_path(std::path::Path::new(r"D:\Dropbox\prestashop")));
        assert!(EnvPaths::is_cloud_synced_path(std::path::Path::new(r"/Users/jane/Google Drive/My Drive/app")));
        assert!(EnvPaths::is_cloud_synced_path(std::path::Path::new(r"C:\Users\jane\iCloudDrive\app")));
        assert!(!EnvPaths::is_cloud_synced_path(std::path::Path::new(r"C:\Tools\prestashop")));
        assert!(!EnvPaths::is_cloud_synced_path(std::path::Path::new(r"/opt/prestashop")));
    }

    #[test]
    fn test_setup_cache_directory() {
        let temp_dir = std::env::temp_dir().join(format!("test_cache_setup_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        EnvPaths::setup_cache_directory(&temp_dir).unwrap();
        let cache_dir = temp_dir.join("var/cache");
        assert!(cache_dir.exists());

        fs::write(cache_dir.join("test.txt"), "cache content").unwrap();
        assert_eq!(fs::read_to_string(cache_dir.join("test.txt")).unwrap(), "cache content");

        EnvPaths::setup_cache_directory(&temp_dir).unwrap();
        assert!(cache_dir.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_setup_cache_directory_cloud_synced() {
        let temp_dir = std::env::temp_dir().join(format!("test_OneDrive_setup_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let target_dir = EnvPaths::setup_cache_directory(&temp_dir).unwrap();
        let cache_dir = temp_dir.join("var/cache");
        assert!(cache_dir.exists());

        // Writing to cache_dir writes transparently to target_dir
        fs::write(cache_dir.join("demo.txt"), "hello from isolated cache").unwrap();
        assert_eq!(fs::read_to_string(target_dir.join("demo.txt")).unwrap(), "hello from isolated cache");

        // Clean cache directory clears target contents without destroying junction/symlink
        EnvPaths::clean_cache_directory(&temp_dir).unwrap();
        assert!(cache_dir.exists());
        assert!(!target_dir.join("demo.txt").exists());

        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::remove_dir_all(&target_dir);
    }
}


