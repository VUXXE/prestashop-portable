use chrono::Local;
use crossbeam_channel::{Receiver, Sender};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub source: String, // "NGINX", "PHP", "DB", "SYSTEM"
    pub message: String,
    pub formatted_line: String,
}

pub struct LogMonitor {
    running: Arc<AtomicBool>,
    pub receiver: Receiver<LogEntry>,
}

impl LogMonitor {
    pub fn start(logs_dir: PathBuf) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let (sender, receiver) = crossbeam_channel::unbounded();

        let thread_running = running.clone();
        thread::spawn(move || {
            Self::poll_logs(logs_dir, thread_running, sender);
        });

        Self { running, receiver }
    }

    fn poll_logs(logs_dir: PathBuf, running: Arc<AtomicBool>, sender: Sender<LogEntry>) {
        struct FileTracker {
            name: &'static str,
            source: &'static str,
            offset: u64,
        }

        let mut trackers = vec![
            FileTracker {
                name: "nginx_access.log",
                source: "NGINX",
                offset: 0,
            },
            FileTracker {
                name: "nginx_error.log",
                source: "NGINX",
                offset: 0,
            },
            FileTracker {
                name: "php_errors.log",
                source: "PHP",
                offset: 0,
            },
            FileTracker {
                name: "mariadb_error.log",
                source: "DB",
                offset: 0,
            },
        ];

        while running.load(Ordering::Relaxed) {
            for tracker in &mut trackers {
                let file_path = logs_dir.join(tracker.name);
                if let Ok(mut file) = File::open(&file_path) {
                    if let Ok(metadata) = file.metadata() {
                        let len = metadata.len();
                        if len < tracker.offset {
                            // File was truncated or rotated
                            tracker.offset = 0;
                        }

                        if len > tracker.offset {
                            if file.seek(SeekFrom::Start(tracker.offset)).is_ok() {
                                let reader = BufReader::new(&file);
                                for line in reader.lines().map_while(Result::ok) {
                                    if !line.trim().is_empty() {
                                        let now = Local::now().format("%H:%M:%S").to_string();
                                        let tag = match tracker.source {
                                            "NGINX" => "[Nginx]  ",
                                            "PHP" => "[PHP]    ",
                                            "DB" => "[MariaDB]",
                                            _ => "[System] ",
                                        };
                                        let formatted_line = format!("[{}] {} {}", now, tag, line);
                                        let _ = sender.send(LogEntry {
                                            timestamp: now,
                                            source: tracker.source.to_string(),
                                            message: line,
                                            formatted_line,
                                        });
                                    }
                                }
                            }
                            tracker.offset = len;
                        }
                    }
                }
            }

            thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Drop for LogMonitor {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
