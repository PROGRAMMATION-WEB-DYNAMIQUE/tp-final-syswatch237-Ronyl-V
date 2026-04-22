use chrono::Local;
use std::cmp::Ordering;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use sysinfo::System;

#[derive(Debug, Clone)]
struct CpuInfo {
    usage_percent: f32,
    core_count: usize,
}

#[derive(Debug, Clone)]
struct MemInfo {
    total_mb: u64,
    used_mb: u64,
    free_mb: u64,
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_usage: f32,
    memory_mb: u64,
}

#[derive(Debug, Clone)]
struct SystemSnapshot {
    timestamp: String,
    cpu: CpuInfo,
    memory: MemInfo,
    top_processes: Vec<ProcessInfo>,
}

impl fmt::Display for CpuInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CPU global: {:>5.1}% | Coeurs: {}",
            self.usage_percent, self.core_count
        )
    }
}

impl fmt::Display for MemInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RAM: {:>6} MB utilises / {:>6} MB total | libres: {:>6} MB",
            self.used_mb, self.total_mb, self.free_mb
        )
    }
}

impl fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PID {:>6} | {:<26} | CPU {:>5.1}% | MEM {:>6} MB",
            self.pid, self.name, self.cpu_usage, self.memory_mb
        )
    }
}

impl fmt::Display for SystemSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "========== SysWatch ==========")?;
        writeln!(f, "Horodatage: {}", self.timestamp)?;
        writeln!(f, "{}", self.cpu)?;
        writeln!(f, "{}", self.memory)?;
        writeln!(f, "------ Top processus CPU ------")?;
        for process in &self.top_processes {
            writeln!(f, "{}", process)?;
        }
        write!(f, "==============================")
    }
}

#[derive(Debug)]
enum SysWatchError {
    EmptyCpuData,
    SnapshotInitFailed(String),
}

impl fmt::Display for SysWatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SysWatchError::EmptyCpuData => write!(f, "Aucune donnee CPU disponible"),
            SysWatchError::SnapshotInitFailed(msg) => write!(f, "Erreur de collecte: {}", msg),
        }
    }
}

impl std::error::Error for SysWatchError {}

fn to_mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

fn ascii_bar(percent: f32, width: usize) -> String {
    let pct = percent.clamp(0.0, 100.0);
    let filled = ((pct / 100.0) * width as f32).round() as usize;
    (0..width)
        .map(|i| if i < filled { '#' } else { '-' })
        .collect()
}

fn collect_snapshot() -> Result<SystemSnapshot, SysWatchError> {
    let mut sys = System::new_all();

    // Deux refresh pour obtenir une mesure CPU plus stable.
    sys.refresh_all();
    thread::sleep(Duration::from_millis(300));
    sys.refresh_all();

    let core_count = sys.cpus().len();
    if core_count == 0 {
        return Err(SysWatchError::EmptyCpuData);
    }

    let cpu = CpuInfo {
        usage_percent: sys.global_cpu_info().cpu_usage(),
        core_count,
    };

    let memory = MemInfo {
        total_mb: to_mb(sys.total_memory()),
        used_mb: to_mb(sys.used_memory()),
        free_mb: to_mb(sys.free_memory()),
    };

    let mut top_processes: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|proc| ProcessInfo {
            pid: proc.pid().as_u32(),
            name: proc.name().to_string(),
            cpu_usage: proc.cpu_usage(),
            memory_mb: to_mb(proc.memory()),
        })
        .collect();

    top_processes.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(Ordering::Equal)
    });
    top_processes.truncate(5);

    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    if timestamp.is_empty() {
        return Err(SysWatchError::SnapshotInitFailed(
            "horodatage indisponible".to_string(),
        ));
    }

    Ok(SystemSnapshot {
        timestamp,
        cpu,
        memory,
        top_processes,
    })
}

fn format_response(snapshot: &SystemSnapshot, command: &str) -> String {
    let cmd = command.trim().to_ascii_lowercase();

    match cmd.as_str() {
        "cpu" => {
            let bar = ascii_bar(snapshot.cpu.usage_percent, 20);
            format!(
                "[CPU]\n{}\n[{}] {:>5.1}%\n",
                snapshot.cpu, bar, snapshot.cpu.usage_percent
            )
        }
        "mem" => {
            let percent = if snapshot.memory.total_mb == 0 {
                0.0
            } else {
                (snapshot.memory.used_mb as f32 / snapshot.memory.total_mb as f32) * 100.0
            };
            let bar = ascii_bar(percent, 20);
            format!("[MEM]\n{}\n[{}] {:>5.1}%\n", snapshot.memory, bar, percent)
        }
        "ps" => {
            let body = snapshot
                .top_processes
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{:>2}. {}", i + 1, p))
                .collect::<Vec<_>>()
                .join("\n");
            format!("[PS]\n{}\n", body)
        }
        "all" | "" => format!("{}\n", snapshot),
        "help" => {
            "Commandes: cpu | mem | ps | all | help | quit\n"
                .to_string()
        }
        "quit" => "BYE\n".to_string(),
        _ => format!(
            "Commande inconnue: '{}'. Tape 'help'.\n",
            command.trim()
        ),
    }
}

fn log_event(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let line = format!("[{}] {}\n", timestamp, message);

    print!("{}", line);

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("syswatch.log")
    {
        let _ = file.write_all(line.as_bytes());
    }
}

fn snapshot_refresher(shared_snapshot: Arc<Mutex<SystemSnapshot>>) {
    loop {
        thread::sleep(Duration::from_secs(5));
        match collect_snapshot() {
            Ok(new_snapshot) => {
                if let Ok(mut snapshot) = shared_snapshot.lock() {
                    *snapshot = new_snapshot;
                }
            }
            Err(err) => {
                log_event(&format!("[refresh] echec: {}", err));
            }
        }
    }
}

fn handle_client(mut stream: TcpStream, shared_snapshot: Arc<Mutex<SystemSnapshot>>) {
    let peer = stream
        .peer_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "inconnu".to_string());

    log_event(&format!("[+] connexion {}", peer));

    let welcome =
        "Bienvenue sur SysWatch\nTape 'help' pour les commandes\n> ";
    let _ = stream.write_all(welcome.as_bytes());

    let reader_stream = match stream.try_clone() {
        Ok(clone) => clone,
        Err(err) => {
            log_event(&format!("[{}] erreur clone stream: {}", peer, err));
            return;
        }
    };

    let reader = BufReader::new(reader_stream);

    for line in reader.lines() {
        match line {
            Ok(command) => {
                let command = command.trim().to_string();
                if command.is_empty() {
                    let _ = stream.write_all(b"> ");
                    continue;
                }

                log_event(&format!("[{}] cmd='{}'", peer, command));

                if command.eq_ignore_ascii_case("quit") {
                    let _ = stream.write_all(b"BYE\n");
                    break;
                }

                let response = match shared_snapshot.lock() {
                    Ok(snapshot) => format_response(&snapshot, &command),
                    Err(_) => "Erreur interne: verrou snapshot indisponible\n".to_string(),
                };

                let _ = stream.write_all(response.as_bytes());

                let _ = stream.write_all(b"> ");
            }
            Err(err) => {
                log_event(&format!("[{}] erreur lecture: {}", peer, err));
                break;
            }
        }
    }

    log_event(&format!("[-] deconnexion {}", peer));
}

fn main() {
    println!("Demarrage SysWatch...");

    let initial_snapshot = match collect_snapshot() {
        Ok(snapshot) => snapshot,
        Err(err) => {
            eprintln!("Erreur initialisation metrics: {}", err);
            return;
        }
    };

    println!("Snapshot initial:\n{}\n", initial_snapshot);

    let shared_snapshot = Arc::new(Mutex::new(initial_snapshot));

    {
        let refresher_snapshot = Arc::clone(&shared_snapshot);
        thread::spawn(move || snapshot_refresher(refresher_snapshot));
    }

    let listener = match TcpListener::bind("0.0.0.0:7878") {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("Impossible d'ouvrir le port 7878: {}", err);
            return;
        }
    };

    println!("Serveur TCP en ecoute sur 0.0.0.0:7878");
    println!("Test client: telnet 127.0.0.1 7878");

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let snapshot_for_client = Arc::clone(&shared_snapshot);
                thread::spawn(move || handle_client(stream, snapshot_for_client));
            }
            Err(err) => {
                log_event(&format!("erreur connexion entrante: {}", err));
            }
        }
    }
}