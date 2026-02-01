import re

# Read the file
with open('src/main.rs', 'r') as f:
    content = f.read()

# New input schemas for missing notebook tools
new_schemas = '''
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CompressInput {
    #[schemars(description = "Text to compress")]
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DecompressInput {
    #[schemars(description = "Base64 compressed data")]
    pub data: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkflowAddInput {
    #[schemars(description = "Workflow name")]
    pub name: String,
    #[schemars(description = "Steps (pipe-separated)")]
    pub steps: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkflowRecallInput {
    #[schemars(description = "Workflow name or ID")]
    pub name_or_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchIdsInput {
    #[schemars(description = "Comma-separated note IDs")]
    pub ids: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchTagInput {
    #[schemars(description = "Comma-separated note IDs")]
    pub ids: String,
    #[schemars(description = "Tags to add")]
    pub tags: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExportInput {
    #[schemars(description = "Format: json, markdown")]
    pub format: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlaybookSearchInput {
    #[schemars(description = "Search query")]
    pub query: String,
    #[schemars(description = "Max results")]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScreenshotInput {
    #[schemars(description = "Window title (optional, captures full screen if empty)")]
    pub window: Option<String>,
    #[schemars(description = "Region x,y,w,h (optional)")]
    pub region: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WebScreenshotInput {
    #[schemars(description = "URL to screenshot")]
    pub url: String,
    #[schemars(description = "Device preset: desktop, mobile, tablet")]
    pub device: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NavigateInput {
    #[schemars(description = "URL to navigate to")]
    pub url: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClickInput {
    #[schemars(description = "CSS selector")]
    pub selector: Option<String>,
    #[schemars(description = "X coordinate")]
    pub x: Option<i32>,
    #[schemars(description = "Y coordinate")]
    pub y: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TypeInput {
    #[schemars(description = "CSS selector")]
    pub selector: String,
    #[schemars(description = "Text to type")]
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScrapeInput {
    #[schemars(description = "URL to scrape")]
    pub url: String,
    #[schemars(description = "CSS selector")]
    pub selector: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PdfInput {
    #[schemars(description = "URL to convert")]
    pub url: String,
    #[schemars(description = "Output path")]
    pub output: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecordInput {
    #[schemars(description = "Window title (optional)")]
    pub window: Option<String>,
    #[schemars(description = "Duration in seconds")]
    pub duration: Option<i32>,
}
'''

# Missing notebook tools + visionbook tools
new_tools = '''
    // ============== MISSING NOTEBOOK TOOLS ==============

    #[tool(description = "Compress text (zstd)")]
    async fn notebook_compress(&self, Parameters(input): Parameters<CompressInput>) -> String {
        use notebook_core::compression;
        match compression::compress_text(&input.text) {
            Ok(data) => format!("Compressed|Size: {} -> {} bytes", input.text.len(), data.len()),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Decompress text")]
    async fn notebook_decompress(&self, Parameters(input): Parameters<DecompressInput>) -> String {
        use notebook_core::compression;
        match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &input.data) {
            Ok(bytes) => match compression::decompress_text(&bytes) {
                Ok(text) => text,
                Err(e) => format!("Error: {}", e),
            },
            Err(e) => format!("Base64 error: {}", e),
        }
    }

    #[tool(description = "Start a new session")]
    async fn notebook_start_session(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_or_create_session() {
            Ok(id) => format!("Session #{}", id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Add a workflow")]
    async fn workflow_add(&self, Parameters(input): Parameters<WorkflowAddInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        // Store as a special note with workflow tag
        let content = format!("WORKFLOW: {}\\n{}", input.name, input.steps.replace("|", "\\n"));
        let mut note = notebook_core::Note::new(content, vec!["workflow".to_string(), input.name.clone()]);
        match notebook.remember(&note) {
            Ok(id) => format!("Workflow '{}' saved|ID: {}", input.name, id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Recall a workflow")]
    async fn workflow_recall(&self, Parameters(input): Parameters<WorkflowRecallInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        // Search for workflow
        match notebook.recall(Some(&format!("WORKFLOW: {}", input.name_or_id)), 1, false) {
            Ok(results) => {
                if results.is_empty() { format!("Workflow '{}' not found", input.name_or_id) }
                else { results[0].note.content.clone() }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "List workflows")]
    async fn workflow_list(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.recall(Some("WORKFLOW:"), input.limit.unwrap_or(20), false) {
            Ok(results) => {
                if results.is_empty() { "No workflows".into() }
                else { results.iter().map(|r| r.note.tags.get(1).cloned().unwrap_or_else(|| "?".into())).collect::<Vec<_>>().join("|") }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Delete multiple notes")]
    async fn batch_delete(&self, Parameters(input): Parameters<BatchIdsInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let ids: Vec<i64> = input.ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let mut deleted = 0;
        for id in &ids {
            if notebook.delete_note(*id).unwrap_or(false) { deleted += 1; }
        }
        format!("Deleted {}/{}", deleted, ids.len())
    }

    #[tool(description = "Pin multiple notes")]
    async fn batch_pin(&self, Parameters(input): Parameters<BatchIdsInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let ids: Vec<i64> = input.ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let mut pinned = 0;
        for id in &ids {
            if notebook.pin_note(*id).unwrap_or(false) { pinned += 1; }
        }
        format!("Pinned {}/{}", pinned, ids.len())
    }

    #[tool(description = "Unpin multiple notes")]
    async fn batch_unpin(&self, Parameters(input): Parameters<BatchIdsInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let ids: Vec<i64> = input.ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let mut unpinned = 0;
        for id in &ids {
            if notebook.unpin_note(*id).unwrap_or(false) { unpinned += 1; }
        }
        format!("Unpinned {}/{}", unpinned, ids.len())
    }

    #[tool(description = "Add tags to multiple notes")]
    async fn batch_tag(&self, Parameters(input): Parameters<BatchTagInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let ids: Vec<i64> = input.ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let tags: Vec<String> = input.tags.split(',').map(|s| s.trim().to_string()).collect();
        let mut tagged = 0;
        for id in &ids {
            if notebook.add_tags(*id, &tags).unwrap_or(false) { tagged += 1; }
        }
        format!("Tagged {}/{}", tagged, ids.len())
    }

    #[tool(description = "Export notes to JSON")]
    async fn notebook_export(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_notes(input.limit.unwrap_or(100)) {
            Ok(notes) => {
                let export: Vec<_> = notes.iter().map(|n| serde_json::json!({
                    "id": n.id, "content": n.content, "tags": n.tags, "pinned": n.pinned
                })).collect();
                serde_json::to_string(&export).unwrap_or_else(|e| format!("Error: {}", e))
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Search playbook strategies")]
    async fn playbook_search(&self, Parameters(input): Parameters<PlaybookSearchInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        // Search strategies by content
        match notebook.list_strategies(input.limit.unwrap_or(10)) {
            Ok(list) => {
                let filtered: Vec<_> = list.iter()
                    .filter(|(_, title, ctx, _, _)| title.to_lowercase().contains(&input.query.to_lowercase()) || ctx.to_lowercase().contains(&input.query.to_lowercase()))
                    .collect();
                if filtered.is_empty() { "No matches".into() }
                else { filtered.iter().map(|(id, t, _, s, _)| format!("{}:{:.1}:{}", id, s, t)).collect::<Vec<_>>().join("|") }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Playbook statistics")]
    async fn playbook_stats(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let strategies = notebook.list_strategies(1000).map(|l| l.len()).unwrap_or(0);
        let insights = notebook.list_insights(1000).map(|l| l.len()).unwrap_or(0);
        let patterns = notebook.list_patterns(1000).map(|l| l.len()).unwrap_or(0);
        format!("Strategies:{}|Insights:{}|Patterns:{}", strategies, insights, patterns)
    }

    // ============== VISIONBOOK TOOLS ==============

    #[tool(description = "Take a screenshot (Windows)")]
    async fn screenshot(&self, Parameters(input): Parameters<ScreenshotInput>) -> String {
        #[cfg(windows)]
        {
            use std::path::PathBuf;
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            let filename = format!("screenshot_{}.png", timestamp);
            let path = dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".ai-foundation")
                .join("screenshots");
            std::fs::create_dir_all(&path).ok();
            let filepath = path.join(&filename);

            // Use Windows GDI for screenshot
            match capture_screen_windows(filepath.to_str().unwrap_or("screenshot.png")) {
                Ok(_) => format!("Screenshot saved|Path: {}", filepath.display()),
                Err(e) => format!("Error: {}", e),
            }
        }
        #[cfg(not(windows))]
        {
            "Screenshot only available on Windows".to_string()
        }
    }

    #[tool(description = "List windows")]
    async fn list_windows(&self) -> String {
        #[cfg(windows)]
        {
            match list_windows_windows() {
                Ok(windows) => {
                    if windows.is_empty() { "No windows".into() }
                    else { windows.join("|") }
                },
                Err(e) => format!("Error: {}", e),
            }
        }
        #[cfg(not(windows))]
        {
            "Window listing only available on Windows".to_string()
        }
    }

    #[tool(description = "List monitors")]
    async fn list_monitors(&self) -> String {
        #[cfg(windows)]
        {
            match list_monitors_windows() {
                Ok(monitors) => {
                    if monitors.is_empty() { "No monitors".into() }
                    else { monitors.join("|") }
                },
                Err(e) => format!("Error: {}", e),
            }
        }
        #[cfg(not(windows))]
        {
            "Monitor listing only available on Windows".to_string()
        }
    }

    #[tool(description = "Web screenshot (requires browser)")]
    async fn web_screenshot(&self, Parameters(input): Parameters<WebScreenshotInput>) -> String {
        format!("Web screenshot not yet implemented|URL: {}", input.url)
    }

    #[tool(description = "Navigate browser")]
    async fn browser_navigate(&self, Parameters(input): Parameters<NavigateInput>) -> String {
        format!("Browser navigation not yet implemented|URL: {}", input.url)
    }

    #[tool(description = "Click element")]
    async fn browser_click(&self, Parameters(input): Parameters<ClickInput>) -> String {
        format!("Browser click not yet implemented|Selector: {:?}", input.selector)
    }

    #[tool(description = "Type text")]
    async fn browser_type(&self, Parameters(input): Parameters<TypeInput>) -> String {
        format!("Browser type not yet implemented|Selector: {}", input.selector)
    }

    #[tool(description = "Scrape webpage")]
    async fn web_scrape(&self, Parameters(input): Parameters<ScrapeInput>) -> String {
        format!("Web scraping not yet implemented|URL: {}", input.url)
    }

    #[tool(description = "Generate PDF from URL")]
    async fn web_pdf(&self, Parameters(input): Parameters<PdfInput>) -> String {
        format!("PDF generation not yet implemented|URL: {}", input.url)
    }

    #[tool(description = "Record screen")]
    async fn screen_record(&self, Parameters(input): Parameters<RecordInput>) -> String {
        format!("Screen recording not yet implemented|Duration: {:?}s", input.duration)
    }

    #[tool(description = "List device presets")]
    async fn list_devices(&self) -> String {
        "desktop:1920x1080|mobile:375x812|tablet:768x1024".to_string()
    }

    #[tool(description = "Visionbook version")]
    async fn visionbook_version(&self) -> String {
        "visionbook 1.0.0|Platform: Windows|Features: screenshot, list_windows, list_monitors".to_string()
    }
'''

# Windows helper functions to add after imports
windows_helpers = '''
// Windows screenshot helpers
#[cfg(windows)]
fn capture_screen_windows(output_path: &str) -> anyhow::Result<()> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::*;
    use std::ptr::null_mut;

    unsafe {
        let hdc_screen = GetDC(HWND(0));
        let hdc_mem = CreateCompatibleDC(hdc_screen);

        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);

        let hbitmap = CreateCompatibleBitmap(hdc_screen, width, height);
        let old_bitmap = SelectObject(hdc_mem, hbitmap);

        BitBlt(hdc_mem, 0, 0, width, height, hdc_screen, 0, 0, SRCCOPY);

        // Save bitmap to file
        let mut bmp_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // Top-down
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD::default()],
        };

        let row_size = ((width * 3 + 3) / 4) * 4;
        let image_size = (row_size * height) as usize;
        let mut pixels = vec![0u8; image_size];

        GetDIBits(hdc_mem, hbitmap, 0, height as u32, Some(pixels.as_mut_ptr() as *mut _), &mut bmp_info, DIB_RGB_COLORS);

        // Write BMP file
        let file_header_size = 14u32;
        let info_header_size = 40u32;
        let file_size = file_header_size + info_header_size + image_size as u32;

        let mut file = std::fs::File::create(output_path)?;
        use std::io::Write;
        file.write_all(&[0x42, 0x4D])?; // BM
        file.write_all(&file_size.to_le_bytes())?;
        file.write_all(&[0, 0, 0, 0])?; // Reserved
        file.write_all(&(file_header_size + info_header_size).to_le_bytes())?;
        file.write_all(&info_header_size.to_le_bytes())?;
        file.write_all(&width.to_le_bytes())?;
        file.write_all(&height.to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?; // Planes
        file.write_all(&24u16.to_le_bytes())?; // Bits
        file.write_all(&[0; 24])?; // Rest of header
        file.write_all(&pixels)?;

        // Cleanup
        SelectObject(hdc_mem, old_bitmap);
        DeleteObject(hbitmap);
        DeleteDC(hdc_mem);
        ReleaseDC(HWND(0), hdc_screen);
    }
    Ok(())
}

#[cfg(windows)]
fn list_windows_windows() -> anyhow::Result<Vec<String>> {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::*;
    use std::sync::Mutex;

    static WINDOWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    unsafe extern "system" fn enum_callback(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut title = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut title);
        if len > 0 && IsWindowVisible(hwnd).as_bool() {
            let title_str = String::from_utf16_lossy(&title[..len as usize]);
            if !title_str.is_empty() {
                WINDOWS.lock().unwrap().push(title_str);
            }
        }
        BOOL(1)
    }

    WINDOWS.lock().unwrap().clear();
    unsafe { EnumWindows(Some(enum_callback), LPARAM(0)).ok(); }
    Ok(WINDOWS.lock().unwrap().clone())
}

#[cfg(windows)]
fn list_monitors_windows() -> anyhow::Result<Vec<String>> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Foundation::*;
    use std::sync::Mutex;

    static MONITORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    unsafe extern "system" fn enum_callback(hmonitor: HMONITOR, _hdc: HDC, _rect: *mut RECT, _: LPARAM) -> BOOL {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(hmonitor, &mut info).as_bool() {
            let w = info.rcMonitor.right - info.rcMonitor.left;
            let h = info.rcMonitor.bottom - info.rcMonitor.top;
            MONITORS.lock().unwrap().push(format!("{}x{}", w, h));
        }
        BOOL(1)
    }

    MONITORS.lock().unwrap().clear();
    unsafe { EnumDisplayMonitors(HDC(0), None, Some(enum_callback), LPARAM(0)).ok(); }
    Ok(MONITORS.lock().unwrap().clone())
}

'''

# Insert schemas after TaskUpdateInput
schema_marker = "pub struct TaskUpdateInput {"
if schema_marker in content:
    idx = content.find(schema_marker)
    end_idx = content.find("}\n", idx) + 2
    content = content[:end_idx] + new_schemas + content[end_idx:]
    print("Inserted schemas")
else:
    print("ERROR: Schema marker not found")

# Insert Windows helpers after the imports section (before // INPUT SCHEMAS)
helper_marker = "// ============================================================================\n// INPUT SCHEMAS"
if helper_marker in content:
    content = content.replace(helper_marker, windows_helpers + "\n" + helper_marker)
    print("Inserted Windows helpers")
else:
    print("ERROR: Helper marker not found")

# Insert tools before closing impl brace
tool_marker = "\n}\n\n#[tool_handler]"
if tool_marker in content:
    content = content.replace(tool_marker, new_tools + "\n}\n\n#[tool_handler]")
    print("Inserted tools")
else:
    print("ERROR: Tool marker not found")

with open('src/main.rs', 'w') as f:
    f.write(content)

print("Done!")
