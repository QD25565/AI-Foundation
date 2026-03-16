//! MCP HTTP Gateway - Windows Service for AI Foundation remote access
//!
//! Enables any web-based AI to connect to AI-Foundation via HTTP MCP protocol.
//! Runs ai-foundation-mcp-http.exe and optionally cloudflared tunnel as a background service.
//!
//! Originally built for Cove (Claude Web) by QD and Lyra, Dec 2025.
//! Generalized to support any HTTP/internet-based AI.
//!
//! Install:   mcp-http-gateway.exe install
//! Uninstall: mcp-http-gateway.exe uninstall
//! Start:     mcp-http-gateway.exe start (or via Services app)
//! Stop:      mcp-http-gateway.exe stop
//! Run:       mcp-http-gateway.exe run (direct mode for testing)

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

const SERVICE_NAME: &str = "MCPHttpGateway";
const SERVICE_DISPLAY_NAME: &str = "AI Foundation MCP HTTP Gateway";
const SERVICE_DESCRIPTION: &str = "HTTP gateway for MCP - enables web-based AIs to connect to AI-Foundation";

// Paths - adjust if needed
fn mcp_server_path() -> PathBuf {
    std::env::var("MCP_HTTP_SERVER_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".ai-foundation")
                .join("bin")
                .join("ai-foundation-mcp-http.exe")
        })
}
const CLOUDFLARED: &str = r"C:\Program Files (x86)\cloudflared\cloudflared.exe";
const PORT: u16 = 8080;

define_windows_service!(ffi_service_main, service_main);

fn main() -> anyhow::Result<()> {
    // Check command line args
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "install" => return install_service(),
            "uninstall" => return uninstall_service(),
            "start" => return start_service(),
            "stop" => return stop_service(),
            "run" => {
                // Run directly (for testing)
                return run_services_directly();
            }
            _ => {
                println!("MCP HTTP Gateway - AI Foundation Remote Access");
                println!();
                println!("Usage:");
                println!("  mcp-http-gateway install   - Install as Windows Service");
                println!("  mcp-http-gateway uninstall - Remove Windows Service");
                println!("  mcp-http-gateway start     - Start the service");
                println!("  mcp-http-gateway stop      - Stop the service");
                println!("  mcp-http-gateway run       - Run directly (for testing)");
                return Ok(());
            }
        }
    }

    // Running as service
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        eprintln!("Service error: {}", e);
    }
}

fn run_service() -> anyhow::Result<()> {
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    // Report running
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // Start both processes immediately — cloudflared retries backend connections automatically.
    // No startup ordering required; no sleep.
    let mcp_process = start_mcp_server()?;
    let tunnel_process = start_cloudflared()?;

    // Event-driven supervisor — zero polling.
    // Each child gets a watcher thread that blocks on wait() (OS-level, zero CPU).
    // Shutdown signal forwarded from service control handler.
    // All events merge into a single channel.
    // PIDs tracked via Arc<Mutex> for clean shutdown.
    enum SupervisorEvent {
        Shutdown,
        McpExited,
        TunnelExited,
    }

    let (event_tx, event_rx) = mpsc::channel();
    let mcp_pid: Arc<Mutex<u32>> = Arc::new(Mutex::new(mcp_process.id()));
    let tunnel_pid: Arc<Mutex<u32>> = Arc::new(Mutex::new(tunnel_process.id()));

    // Forward shutdown signal
    let tx_shutdown = event_tx.clone();
    std::thread::spawn(move || {
        let _ = shutdown_rx.recv();
        let _ = tx_shutdown.send(SupervisorEvent::Shutdown);
    });

    // Spawn watcher — blocks on wait() (OS-level, zero CPU)
    fn spawn_watcher(
        mut child: Child,
        tx: mpsc::Sender<SupervisorEvent>,
        event: SupervisorEvent,
    ) {
        std::thread::spawn(move || {
            let _ = child.wait();
            let _ = tx.send(event);
        });
    }

    spawn_watcher(mcp_process, event_tx.clone(), SupervisorEvent::McpExited);
    spawn_watcher(tunnel_process, event_tx.clone(), SupervisorEvent::TunnelExited);

    // Single event loop — blocks on recv(), zero CPU, zero polling
    loop {
        match event_rx.recv() {
            Ok(SupervisorEvent::Shutdown) | Err(_) => break,
            Ok(SupervisorEvent::McpExited) => {
                match start_mcp_server() {
                    Ok(child) => {
                        *mcp_pid.lock().unwrap() = child.id();
                        spawn_watcher(child, event_tx.clone(), SupervisorEvent::McpExited);
                    }
                    Err(e) => {
                        eprintln!("Failed to restart MCP server: {e}");
                        break;
                    }
                }
            }
            Ok(SupervisorEvent::TunnelExited) => {
                match start_cloudflared() {
                    Ok(child) => {
                        *tunnel_pid.lock().unwrap() = child.id();
                        spawn_watcher(child, event_tx.clone(), SupervisorEvent::TunnelExited);
                    }
                    Err(e) => {
                        eprintln!("Failed to restart cloudflared: {e}");
                        break;
                    }
                }
            }
        }
    }

    // Kill child processes by PID on shutdown
    let _ = Command::new("taskkill")
        .args(["/F", "/PID", &mcp_pid.lock().unwrap().to_string()])
        .output();
    let _ = Command::new("taskkill")
        .args(["/F", "/PID", &tunnel_pid.lock().unwrap().to_string()])
        .output();

    // Report stopped
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}

fn start_mcp_server() -> anyhow::Result<Child> {
    // AI_ID can be customized via env var, defaults to "mcp-gateway"
    let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "mcp-gateway".to_string());

    let child = Command::new(mcp_server_path())
        .args(["--port", &PORT.to_string()])
        .env("AI_ID", &ai_id)
        .env("POSTGRES_URL", std::env::var("POSTGRES_URL").unwrap_or_else(|_| "postgresql://ai_foundation:changeme@127.0.0.1:15432/ai_foundation".to_string()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(child)
}

fn start_cloudflared() -> anyhow::Result<Child> {
    let child = Command::new(CLOUDFLARED)
        .args(["tunnel", "--config", r"C:\ProgramData\cloudflared\config.yml", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(child)
}

fn run_services_directly() -> anyhow::Result<()> {
    println!("Starting MCP HTTP Gateway (direct mode)...");
    println!("MCP Server: {:?}", mcp_server_path());
    println!("Cloudflared: {}", CLOUDFLARED);
    println!();

    let mcp = start_mcp_server()?;
    let mcp_pid = mcp.id();
    println!("MCP server started (PID: {})", mcp_pid);

    // Start tunnel immediately — cloudflared retries backend connections automatically.
    let tunnel = start_cloudflared()?;
    let tunnel_pid = tunnel.id();
    println!("Cloudflare tunnel started (PID: {})", tunnel_pid);

    println!();
    println!("MCP HTTP Gateway running");
    println!("Endpoint: https://mcp.myappapp.org/mcp (or your configured tunnel)");
    println!("Config: C:\\ProgramData\\cloudflared\\config.yml");
    println!("Press Ctrl+C to stop...");

    // Wait for either process to exit — zero polling.
    // Each thread blocks on wait() (OS-level, zero CPU), signals via channel.
    let (tx, rx) = mpsc::channel();
    let tx_mcp = tx.clone();
    std::thread::spawn(move || {
        let mut child = mcp;
        let status = child.wait();
        let _ = tx_mcp.send(("MCP server", status, tunnel_pid));
    });
    std::thread::spawn(move || {
        let mut child = tunnel;
        let status = child.wait();
        let _ = tx.send(("Cloudflared", status, mcp_pid));
    });

    // Block on whichever exits first — OS-level wait, zero CPU
    match rx.recv() {
        Ok((name, Ok(status), other_pid)) => {
            println!("{} exited with: {:?}", name, status);
            // Kill the other process
            let _ = Command::new("taskkill")
                .args(["/F", "/PID", &other_pid.to_string()])
                .output();
        }
        Ok((name, Err(e), other_pid)) => {
            println!("{} wait failed: {:?}", name, e);
            let _ = Command::new("taskkill")
                .args(["/F", "/PID", &other_pid.to_string()])
                .output();
        }
        Err(_) => {
            println!("Process monitor channels closed");
        }
    }

    Ok(())
}

fn install_service() -> anyhow::Result<()> {
    use windows_service::service::{ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CREATE_SERVICE,
    )?;

    let service_binary = std::env::current_exe()?;

    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: service_binary,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };

    let service = manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG)?;
    service.set_description(SERVICE_DESCRIPTION)?;

    println!("Service '{}' installed successfully!", SERVICE_DISPLAY_NAME);
    println!();
    println!("To start: mcp-http-gateway start");
    println!("Or use Windows Services app (services.msc)");

    Ok(())
}

fn uninstall_service() -> anyhow::Result<()> {
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )?;

    let service = manager.open_service(SERVICE_NAME, ServiceAccess::DELETE)?;
    service.delete()?;

    println!("Service '{}' uninstalled successfully!", SERVICE_DISPLAY_NAME);

    Ok(())
}

fn start_service() -> anyhow::Result<()> {
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )?;

    let service = manager.open_service(SERVICE_NAME, ServiceAccess::START)?;
    service.start::<String>(&[])?;

    println!("Service '{}' started!", SERVICE_DISPLAY_NAME);
    println!("Web-based AIs can now connect via HTTP MCP");

    Ok(())
}

fn stop_service() -> anyhow::Result<()> {
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )?;

    let service = manager.open_service(SERVICE_NAME, ServiceAccess::STOP)?;
    service.stop()?;

    println!("Service '{}' stopped.", SERVICE_DISPLAY_NAME);

    Ok(())
}
