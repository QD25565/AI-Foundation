// Visionbook CLI - Visual tools for AI agents
// Part of AI-Foundation

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use visionbook_core::{
    ScreenshotCapture, BrowserSession, PdfGenerator, VideoRecorder, DeviceEmulator,
    ScreenshotOptions, CaptureTarget, ImageFormat, NavigationOptions, ElementSelector,
    PdfOptions, VideoOptions, parse_device, list_devices,
    VisualMemory,
};

#[derive(Parser)]
#[command(name = "visionbook")]
#[command(version = "1.0.0")]
#[command(about = "Visual tools for AI agents - screenshots, browser automation, video recording", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Capture a screenshot of screen, window, or region
    #[command(
        alias = "snap",
        alias = "capture",
        alias = "shot",
        alias = "grab",
        alias = "ss"
    )]
    Screenshot {
        /// Output file path (e.g., screenshot.png, captures/ui.png)
        #[arg(value_name = "OUTPUT")]
        output_positional: Option<PathBuf>,

        /// Window title to capture (e.g., "Chrome", "Deep Net", "VS Code")
        #[arg(long = "window", value_name = "TITLE")]
        window: Option<String>,

        /// Monitor index to capture, 0-based (e.g., 0, 1, 2)
        #[arg(long = "monitor", value_name = "INDEX")]
        monitor: Option<usize>,

        /// Capture specific region: x,y,width,height (e.g., 0,0,800,600)
        #[arg(long = "region", value_name = "X,Y,W,H")]
        region: Option<String>,

        /// Image format: png, jpeg, webp (default: png)
        #[arg(long = "format", default_value = "png")]
        format: String,

        /// Auto-optimize: resize to AI's optimal resolution from vision profile
        #[arg(long = "auto")]
        auto_optimize: bool,

        /// Create thumbnail instead of full-res (with --auto)
        #[arg(long = "thumbnail")]
        thumbnail: bool,

        // Hidden flag versions for AIs that try short flags
        #[arg(short = 'w', hide = true)]
        window_short: Option<String>,
        #[arg(short = 'm', hide = true)]
        monitor_short: Option<usize>,
        #[arg(short = 'r', hide = true)]
        region_short: Option<String>,
        #[arg(short = 'f', hide = true)]
        format_short: Option<String>,
        #[arg(short = 'a', hide = true)]
        auto_short: bool,
        #[arg(short = 't', hide = true)]
        thumbnail_short: bool,
        #[arg(short = 'o', long = "output", hide = true)]
        output: Option<PathBuf>,
    },

    /// Navigate to a URL in the browser
    #[command(
        alias = "goto",
        alias = "open",
        alias = "visit",
        alias = "browse",
        alias = "load"
    )]
    Navigate {
        /// URL to navigate to (e.g., https://example.com)
        #[arg(value_name = "URL")]
        url_positional: Option<String>,

        /// Wait condition: load, dom, network-idle
        #[arg(short = 'w', long = "wait", default_value = "load")]
        wait: String,

        /// Timeout in milliseconds
        #[arg(short = 't', long = "timeout", default_value = "30000")]
        timeout: u64,

        // Hidden flag version
        #[arg(short = 'u', long = "url", hide = true)]
        url: Option<String>,
    },

    /// Capture screenshot of a web page
    #[command(
        alias = "web-screenshot",
        alias = "web-snap",
        alias = "web-capture",
        alias = "page-shot",
        alias = "webshot"
    )]
    WebScreenshot {
        /// URL to capture (e.g., https://example.com)
        #[arg(value_name = "URL")]
        url_positional: Option<String>,

        /// Output file path (e.g., webpage.png)
        #[arg(value_name = "OUTPUT")]
        output_positional: Option<PathBuf>,

        /// CSS selector for specific element (e.g., "#login-button")
        #[arg(short = 's', long = "selector", value_name = "SELECTOR")]
        selector: Option<String>,

        /// Image format: png, jpeg, webp
        #[arg(short = 'f', long = "format", default_value = "png")]
        format: String,

        /// Mobile device to emulate (e.g., iphone12, pixel7pro)
        #[arg(short = 'd', long = "device", value_name = "DEVICE")]
        device: Option<String>,

        // Hidden flag versions
        #[arg(short = 'u', long = "url", hide = true)]
        url: Option<String>,
        #[arg(short = 'o', long = "output", hide = true)]
        output: Option<PathBuf>,
    },

    /// Click an element on a web page
    #[command(alias = "press", alias = "tap")]
    Click {
        /// URL to navigate to (e.g., https://example.com)
        #[arg(value_name = "URL")]
        url_positional: Option<String>,

        /// CSS selector for element (e.g., "#submit-button")
        #[arg(value_name = "SELECTOR")]
        selector_positional: Option<String>,

        // Hidden flag versions
        #[arg(short = 'u', long = "url", hide = true)]
        url: Option<String>,
        #[arg(short = 's', long = "selector", hide = true)]
        selector: Option<String>,
    },

    /// Type text into an element on a web page
    #[command(alias = "input", alias = "enter", alias = "fill")]
    Type {
        /// URL to navigate to (e.g., https://example.com)
        #[arg(value_name = "URL")]
        url_positional: Option<String>,

        /// CSS selector for element (e.g., "#username")
        #[arg(value_name = "SELECTOR")]
        selector_positional: Option<String>,

        /// Text to type (e.g., "testuser@example.com")
        #[arg(value_name = "TEXT")]
        text_positional: Option<String>,

        // Hidden flag versions
        #[arg(short = 'u', long = "url", hide = true)]
        url: Option<String>,
        #[arg(short = 's', long = "selector", hide = true)]
        selector: Option<String>,
        #[arg(short = 't', long = "text", hide = true)]
        text: Option<String>,
    },

    /// Scrape text content from a web page element
    #[command(alias = "extract", alias = "get-text", alias = "read")]
    Scrape {
        /// URL to navigate to (e.g., https://example.com)
        #[arg(value_name = "URL")]
        url_positional: Option<String>,

        /// CSS selector for element (e.g., ".product-price")
        #[arg(value_name = "SELECTOR")]
        selector_positional: Option<String>,

        // Hidden flag versions
        #[arg(short = 'u', long = "url", hide = true)]
        url: Option<String>,
        #[arg(short = 's', long = "selector", hide = true)]
        selector: Option<String>,
    },

    /// Generate PDF from a web page
    #[command(alias = "export-pdf", alias = "print", alias = "save-pdf")]
    Pdf {
        /// URL to convert to PDF (e.g., https://example.com)
        #[arg(value_name = "URL")]
        url_positional: Option<String>,

        /// Output PDF file path (e.g., page.pdf)
        #[arg(value_name = "OUTPUT")]
        output_positional: Option<PathBuf>,

        /// Use landscape orientation
        #[arg(short = 'l', long = "landscape")]
        landscape: bool,

        /// Print background graphics
        #[arg(short = 'b', long = "background", default_value = "true")]
        background: bool,

        /// Page scale factor (e.g., 0.8, 1.0, 1.5)
        #[arg(long = "scale", default_value = "1.0")]
        scale: f64,

        // Hidden flag versions
        #[arg(short = 'u', long = "url", hide = true)]
        url: Option<String>,
        #[arg(short = 'o', long = "output", hide = true)]
        output: Option<PathBuf>,
    },

    /// Record video of screen or window
    #[command(alias = "rec", alias = "capture-video", alias = "record-screen")]
    Record {
        /// Output video file path (e.g., recording.mp4)
        #[arg(value_name = "OUTPUT")]
        output_positional: Option<PathBuf>,

        /// Window title to record (e.g., "Chrome")
        #[arg(short = 'w', long = "window", value_name = "TITLE")]
        window: Option<String>,

        /// Duration in seconds
        #[arg(short = 'd', long = "duration", default_value = "10")]
        duration: u64,

        /// Frames per second
        #[arg(long = "fps", default_value = "30")]
        fps: u32,

        // Hidden flag version
        #[arg(short = 'o', long = "output", hide = true)]
        output: Option<PathBuf>,
    },

    /// List available windows for capture
    #[command(
        alias = "windows",
        alias = "list-win",
        alias = "lswin",
        alias = "show-windows"
    )]
    ListWindows,

    /// List available monitors for capture
    #[command(
        alias = "monitors",
        alias = "screens",
        alias = "displays",
        alias = "list-mon",
        alias = "lsmon"
    )]
    ListMonitors,

    /// List available mobile device presets for emulation
    #[command(
        alias = "devices",
        alias = "list-dev",
        alias = "lsdev",
        alias = "mobiles"
    )]
    ListDevices,

    /// Attach an image to an Engram note
    #[command(
        alias = "attach-to-note",
        alias = "link",
        alias = "add-visual"
    )]
    Attach {
        /// Note ID to attach the image to (from notebook)
        #[arg(value_name = "NOTE_ID")]
        note_id: u64,

        /// Path to the image file
        #[arg(value_name = "IMAGE_PATH")]
        image_path: PathBuf,

        /// Optional context/caption for the image
        #[arg(short = 'c', long = "context")]
        context: Option<String>,
    },

    /// List recent visual memories
    #[command(
        alias = "visuals",
        alias = "lsv",
        alias = "visual-list"
    )]
    VisualList {
        /// Maximum number of entries to show
        #[arg(short = 'n', long = "limit", default_value = "10")]
        limit: usize,
    },

    /// Get visual memory by ID
    #[command(alias = "vget", alias = "visual-get")]
    VisualGet {
        /// Visual memory ID
        #[arg(value_name = "ID")]
        id: u64,

        /// Extract thumbnail to file
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
    },

    /// Show visual memory statistics
    #[command(alias = "vstats", alias = "visual-stats")]
    VisualStats,

    /// Get visuals attached to a note
    #[command(alias = "note-visuals", alias = "nv")]
    NoteVisuals {
        /// Note ID to get visuals for
        #[arg(value_name = "NOTE_ID")]
        note_id: u64,
    },

    /// Show visionbook version
    #[command(alias = "ver")]
    Version,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize visionbook
    visionbook_core::init()?;

    let cli = Cli::parse();

    match cli.command {
        Commands::Screenshot {
            output_positional,
            window,
            monitor,
            region,
            format,
            auto_optimize,
            thumbnail,
            window_short,
            monitor_short,
            region_short,
            format_short,
            auto_short,
            thumbnail_short,
            output,
        } => {
            let output_path = output_positional.or(output)
                .unwrap_or_else(|| PathBuf::from("screenshot.png"));

            // Merge long and short flag versions
            let window = window.or(window_short);
            let monitor = monitor.or(monitor_short);
            let region = region.or(region_short);
            let format = format_short.unwrap_or(format);
            let auto_optimize = auto_optimize || auto_short;
            let thumbnail = thumbnail || thumbnail_short;

            let image_format = match format.as_str() {
                "jpeg" | "jpg" => ImageFormat::Jpeg,
                "webp" => ImageFormat::WebP,
                _ => ImageFormat::Png,
            };

            // Priority: window > monitor > region > screen
            let target = if let Some(window_title) = window {
                CaptureTarget::Window(window_title)
            } else if let Some(monitor_index) = monitor {
                CaptureTarget::Monitor(monitor_index)
            } else if let Some(region_str) = region {
                let parts: Vec<i32> = region_str
                    .split(',')
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if parts.len() != 4 {
                    eprintln!("Error: Region must be x,y,width,height (e.g., 0,0,800,600)");
                    eprintln!("Hint: visionbook screenshot output.png --region 0,0,800,600");
                    std::process::exit(1);
                }
                CaptureTarget::Region {
                    x: parts[0],
                    y: parts[1],
                    width: parts[2] as u32,
                    height: parts[3] as u32,
                }
            } else {
                CaptureTarget::Screen
            };

            let options = ScreenshotOptions {
                target,
                output_path,
                format: image_format,
                quality: None,
            };

            if auto_optimize {
                // Use auto-optimization with AI vision profile
                ScreenshotCapture::capture_auto_optimized(options, thumbnail).await?;
            } else {
                ScreenshotCapture::capture(options).await?;
            }
        }

        Commands::Navigate {
            url_positional,
            url,
            wait,
            timeout,
        } => {
            let Some(url_str) = url_positional.or(url) else {
                eprintln!("Error: URL is required.");
                eprintln!("Hint: visionbook navigate https://example.com");
                std::process::exit(1);
            };

            let browser = BrowserSession::launch().await?;

            let options = NavigationOptions {
                url: url_str,
                wait_until: match wait.as_str() {
                    "dom" => visionbook_core::WaitCondition::DomContentLoaded,
                    "network-idle" => visionbook_core::WaitCondition::NetworkIdle,
                    _ => visionbook_core::WaitCondition::Load,
                },
                timeout_ms: timeout,
            };

            browser.navigate(options).await?;
            browser.close().await?;
        }

        Commands::WebScreenshot {
            url_positional,
            url,
            output_positional,
            output,
            selector,
            format,
            device,
        } => {
            let Some(url_str) = url_positional.or(url) else {
                eprintln!("Error: URL is required.");
                eprintln!("Hint: visionbook web-screenshot https://example.com output.png");
                std::process::exit(1);
            };
            let output_path = output_positional.or(output)
                .unwrap_or_else(|| PathBuf::from("webpage.png"));

            let image_format = match format.as_str() {
                "jpeg" | "jpg" => ImageFormat::Jpeg,
                "webp" => ImageFormat::WebP,
                _ => ImageFormat::Png,
            };

            let browser = BrowserSession::launch().await?;

            // Apply device emulation if specified
            if let Some(device_name) = device {
                if let Some(mobile_device) = parse_device(&device_name) {
                    let emulator = DeviceEmulator::new(mobile_device);
                    emulator.apply().await?;
                } else {
                    eprintln!("Unknown device: {}. Use 'visionbook list-devices' to see available options.", device_name);
                    std::process::exit(1);
                }
            }

            browser.navigate(NavigationOptions {
                url: url_str,
                ..Default::default()
            }).await?;

            if let Some(selector_str) = selector {
                browser.screenshot_element(
                    &ElementSelector::Css(selector_str),
                    &output_path,
                    image_format,
                ).await?;
            } else {
                browser.screenshot(&output_path, image_format).await?;
            }

            browser.close().await?;
        }

        Commands::Click {
            url_positional,
            url,
            selector_positional,
            selector,
        } => {
            let Some(url_str) = url_positional.or(url) else {
                eprintln!("Error: URL is required.");
                eprintln!("Hint: visionbook click https://example.com \"#button\"");
                std::process::exit(1);
            };
            let Some(selector_str) = selector_positional.or(selector) else {
                eprintln!("Error: CSS selector is required.");
                eprintln!("Hint: visionbook click {} \"#submit-button\"", url_str);
                std::process::exit(1);
            };

            let browser = BrowserSession::launch().await?;
            browser.navigate(NavigationOptions {
                url: url_str,
                ..Default::default()
            }).await?;
            browser.click(&ElementSelector::Css(selector_str)).await?;
            browser.close().await?;
        }

        Commands::Type {
            url_positional,
            url,
            selector_positional,
            selector,
            text_positional,
            text,
        } => {
            let Some(url_str) = url_positional.or(url) else {
                eprintln!("Error: URL is required.");
                eprintln!("Hint: visionbook type https://example.com \"#input\" \"text\"");
                std::process::exit(1);
            };
            let Some(selector_str) = selector_positional.or(selector) else {
                eprintln!("Error: CSS selector is required.");
                eprintln!("Hint: visionbook type {} \"#username\" \"user@example.com\"", url_str);
                std::process::exit(1);
            };
            let Some(text_str) = text_positional.or(text) else {
                eprintln!("Error: Text to type is required.");
                eprintln!("Hint: visionbook type {} \"{}\" \"your text here\"", url_str, selector_str);
                std::process::exit(1);
            };

            let browser = BrowserSession::launch().await?;
            browser.navigate(NavigationOptions {
                url: url_str,
                ..Default::default()
            }).await?;
            browser.type_text(&ElementSelector::Css(selector_str), &text_str).await?;
            browser.close().await?;
        }

        Commands::Scrape {
            url_positional,
            url,
            selector_positional,
            selector,
        } => {
            let Some(url_str) = url_positional.or(url) else {
                eprintln!("Error: URL is required.");
                eprintln!("Hint: visionbook scrape https://example.com \".content\"");
                std::process::exit(1);
            };
            let Some(selector_str) = selector_positional.or(selector) else {
                eprintln!("Error: CSS selector is required.");
                eprintln!("Hint: visionbook scrape {} \".product-price\"", url_str);
                std::process::exit(1);
            };

            let browser = BrowserSession::launch().await?;
            browser.navigate(NavigationOptions {
                url: url_str,
                ..Default::default()
            }).await?;
            let content = browser.scrape(&ElementSelector::Css(selector_str)).await?;
            println!("{}", content);
            browser.close().await?;
        }

        Commands::Pdf {
            url_positional,
            url,
            output_positional,
            output,
            landscape,
            background,
            scale,
        } => {
            let Some(url_str) = url_positional.or(url) else {
                eprintln!("Error: URL is required.");
                eprintln!("Hint: visionbook pdf https://example.com output.pdf");
                std::process::exit(1);
            };
            let output_path = output_positional.or(output)
                .unwrap_or_else(|| PathBuf::from("page.pdf"));

            let options = PdfOptions {
                url: url_str,
                output_path,
                landscape,
                print_background: background,
                scale,
            };

            PdfGenerator::generate(options).await?;
        }

        Commands::Record {
            output_positional,
            output,
            window,
            duration,
            fps,
        } => {
            let output_path = output_positional.or(output)
                .unwrap_or_else(|| PathBuf::from("recording.mp4"));

            let target = if let Some(window_title) = window {
                CaptureTarget::Window(window_title)
            } else {
                CaptureTarget::Screen
            };

            let options = VideoOptions {
                target,
                output_path,
                duration_secs: duration,
                fps,
            };

            VideoRecorder::record(options).await?;
        }

        Commands::ListWindows => {
            let windows = ScreenshotCapture::list_windows()?;
            for window in windows.iter() {
                println!("{}", window);
            }
        }

        Commands::ListMonitors => {
            let monitors = ScreenshotCapture::list_monitors()?;
            for monitor in monitors {
                println!("{}", monitor);
            }
        }

        Commands::ListDevices => {
            let devices = list_devices();
            for (name, description) in devices {
                println!("{}|{}", name, description);
            }
        }

        Commands::Attach { note_id, image_path, context } => {
            let mut vm = VisualMemory::open()
                .map_err(|e| {
                    eprintln!("Error: Failed to open visual memory: {}", e);
                    eprintln!("Hint: Ensure AI_ID environment variable is set");
                    e
                })?;

            let visual_id = vm.attach_to_note(note_id, &image_path, context.as_deref())?;
            println!("Attached|{}|note:{}", visual_id, note_id);
        }

        Commands::VisualList { limit } => {
            let vm = VisualMemory::open()?;
            let visuals = vm.list_recent(limit)?;

            if visuals.is_empty() {
                println!("No visual memories found");
            } else {
                println!("|VISUALS|{}", visuals.len());
                for v in visuals {
                    let note_ref = if v.note_id > 0 {
                        format!("note:{}", v.note_id)
                    } else {
                        "standalone".to_string()
                    };
                    println!("  #{} ({}) [{}x{}] {} | {}",
                        v.id,
                        v.age_string(),
                        v.thumbnail_width,
                        v.thumbnail_height,
                        note_ref,
                        v.context.as_deref().unwrap_or("-")
                    );
                }
            }
        }

        Commands::VisualGet { id, output } => {
            let vm = VisualMemory::open()?;

            match vm.get_visual(id)? {
                Some(v) => {
                    println!("ID:{}", v.id);
                    println!("NoteID:{}", v.note_id);
                    println!("Age:{}", v.age_string());
                    println!("OriginalSize:{}x{}", v.original_width, v.original_height);
                    println!("ThumbnailSize:{}x{}", v.thumbnail_width, v.thumbnail_height);
                    println!("ThumbnailBytes:{}", v.thumbnail_data.len());
                    if let Some(ctx) = &v.context {
                        println!("Context:{}", ctx);
                    }
                    if let Some(path) = &v.original_path {
                        println!("OriginalPath:{}", path);
                    }

                    if let Some(out_path) = output {
                        std::fs::write(&out_path, &v.thumbnail_data)?;
                        println!("ThumbnailSaved:{}", out_path.display());
                    }
                }
                None => {
                    eprintln!("Error: Visual {} not found", id);
                    std::process::exit(1);
                }
            }
        }

        Commands::VisualStats => {
            let vm = VisualMemory::open()?;
            let stats = vm.stats();

            println!("|VISUAL MEMORY STATS|");
            println!("TotalEntries:{}", stats.total_entries);
            println!("ActiveEntries:{}", stats.active_entries);
            println!("NextID:{}", stats.next_id);
        }

        Commands::NoteVisuals { note_id } => {
            let vm = VisualMemory::open()?;
            let visuals = vm.get_visuals_for_note(note_id)?;

            if visuals.is_empty() {
                println!("No visuals attached to note {}", note_id);
            } else {
                println!("|VISUALS FOR NOTE {}|{}", note_id, visuals.len());
                for v in visuals {
                    println!("  #{} ({}) [{}x{}] | {}",
                        v.id,
                        v.age_string(),
                        v.thumbnail_width,
                        v.thumbnail_height,
                        v.context.as_deref().unwrap_or("-")
                    );
                }
            }
        }


        Commands::Version => {
            println!("visionbook v{} | AI-Foundation", visionbook_core::version());
        }
    }

    Ok(())
}
