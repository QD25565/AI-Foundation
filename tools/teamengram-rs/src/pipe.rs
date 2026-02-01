//! Windows Named Pipe Implementation - Direct Windows API
//!
//! This module provides a proper Windows named pipe implementation using
//! the Windows API directly (no tokio abstraction). Full control, full visibility.
//!
//! Architecture:
//! - PipeServer: Creates and listens on a named pipe
//! - PipeClient: Connects to an existing named pipe
//! - Both use synchronous I/O for reliability

#[cfg(windows)]
pub mod windows {
    use std::ffi::OsStr;
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
        ERROR_PIPE_CONNECTED,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FlushFileBuffers,
        OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL,
    };
    use windows_sys::Win32::System::Pipes::{
        CreateNamedPipeW, ConnectNamedPipe, DisconnectNamedPipe,
        PIPE_TYPE_MESSAGE, PIPE_READMODE_MESSAGE,
        PIPE_WAIT, PIPE_UNLIMITED_INSTANCES,
    };

    // These constants are not exported by windows-sys, define them manually
    const GENERIC_READ: u32 = 0x80000000;
    const GENERIC_WRITE: u32 = 0x40000000;
    const PIPE_ACCESS_DUPLEX: u32 = 0x00000003;

    const PIPE_BUFFER_SIZE: u32 = 65536;
    const PIPE_TIMEOUT_MS: u32 = 5000;

    /// Convert a Rust string to a null-terminated wide string for Windows API
    fn to_wide_string(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Windows Named Pipe Server
    ///
    /// Creates a named pipe and waits for client connections.
    /// Uses synchronous I/O for maximum reliability.
    pub struct PipeServer {
        handle: HANDLE,
        pipe_name: String,
    }

    impl PipeServer {
        /// Create a new named pipe server
        ///
        /// pipe_name should be in format: \\.\pipe\pipename
        pub fn create(pipe_name: &str) -> io::Result<Self> {
            let wide_name = to_wide_string(pipe_name);

            let handle = unsafe {
                CreateNamedPipeW(
                    wide_name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    PIPE_BUFFER_SIZE,
                    PIPE_BUFFER_SIZE,
                    PIPE_TIMEOUT_MS,
                    ptr::null_mut(), // Default security
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }

            Ok(Self {
                handle,
                pipe_name: pipe_name.to_string(),
            })
        }

        /// Get the pipe name
        pub fn pipe_name(&self) -> &str {
            &self.pipe_name
        }

        /// Wait for a client to connect
        ///
        /// This blocks until a client connects or an error occurs.
        pub fn wait_for_connection(&self) -> io::Result<()> {
            let result = unsafe { ConnectNamedPipe(self.handle, ptr::null_mut()) };

            if result == 0 {
                let err = unsafe { GetLastError() };
                // ERROR_PIPE_CONNECTED means client already connected - that's fine
                if err != ERROR_PIPE_CONNECTED {
                    return Err(io::Error::from_raw_os_error(err as i32));
                }
            }

            Ok(())
        }

        /// Disconnect the current client (allows accepting a new one)
        pub fn disconnect(&self) -> io::Result<()> {
            let result = unsafe { DisconnectNamedPipe(self.handle) };
            if result == 0 {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }
            Ok(())
        }

        /// Read data from the connected client
        pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
            let mut bytes_read: u32 = 0;
            let result = unsafe {
                ReadFile(
                    self.handle,
                    buf.as_mut_ptr() as *mut _,
                    buf.len() as u32,
                    &mut bytes_read,
                    ptr::null_mut(),
                )
            };

            if result == 0 {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }

            Ok(bytes_read as usize)
        }

        /// Write data to the connected client
        pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
            let mut bytes_written: u32 = 0;
            let result = unsafe {
                WriteFile(
                    self.handle,
                    buf.as_ptr() as *const _,
                    buf.len() as u32,
                    &mut bytes_written,
                    ptr::null_mut(),
                )
            };

            if result == 0 {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }

            Ok(bytes_written as usize)
        }

        /// Flush the pipe buffer
        pub fn flush(&self) -> io::Result<()> {
            let result = unsafe { FlushFileBuffers(self.handle) };
            if result == 0 {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }
            Ok(())
        }

        /// Get the raw handle (for advanced use)
        pub fn handle(&self) -> HANDLE {
            self.handle
        }
    }

    impl Drop for PipeServer {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    // Implement Read trait for PipeServer
    impl Read for PipeServer {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            PipeServer::read(self, buf)
        }
    }

    // Implement Write trait for PipeServer
    impl Write for PipeServer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            PipeServer::write(self, buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            PipeServer::flush(self)
        }
    }

    /// Windows Named Pipe Client
    ///
    /// Connects to an existing named pipe server.
    pub struct PipeClient {
        handle: HANDLE,
    }

    impl PipeClient {
        /// Connect to a named pipe server
        ///
        /// pipe_name should be in format: \\.\pipe\pipename
        pub fn connect(pipe_name: &str) -> io::Result<Self> {
            let wide_name = to_wide_string(pipe_name);

            let handle = unsafe {
                CreateFileW(
                    wide_name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0, // No sharing
                    ptr::null_mut(), // Default security
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    0, // No template (HANDLE is isize, not *mut)
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }

            Ok(Self { handle })
        }

        /// Read data from the server
        pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
            let mut bytes_read: u32 = 0;
            let result = unsafe {
                ReadFile(
                    self.handle,
                    buf.as_mut_ptr() as *mut _,
                    buf.len() as u32,
                    &mut bytes_read,
                    ptr::null_mut(),
                )
            };

            if result == 0 {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }

            Ok(bytes_read as usize)
        }

        /// Write data to the server
        pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
            let mut bytes_written: u32 = 0;
            let result = unsafe {
                WriteFile(
                    self.handle,
                    buf.as_ptr() as *const _,
                    buf.len() as u32,
                    &mut bytes_written,
                    ptr::null_mut(),
                )
            };

            if result == 0 {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }

            Ok(bytes_written as usize)
        }

        /// Flush the pipe buffer
        pub fn flush(&self) -> io::Result<()> {
            let result = unsafe { FlushFileBuffers(self.handle) };
            if result == 0 {
                let err = unsafe { GetLastError() };
                return Err(io::Error::from_raw_os_error(err as i32));
            }
            Ok(())
        }
    }

    impl Drop for PipeClient {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    // Implement Read trait for PipeClient
    impl Read for PipeClient {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            PipeClient::read(self, buf)
        }
    }

    // Implement Write trait for PipeClient
    impl Write for PipeClient {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            PipeClient::write(self, buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            PipeClient::flush(self)
        }
    }

    /// Check if a named pipe exists and is connectable
    pub fn pipe_exists(pipe_name: &str) -> bool {
        match PipeClient::connect(pipe_name) {
            Ok(_) => true,
            Err(_) => false,
        }
    }
}

#[cfg(not(windows))]
pub mod unix {
    use std::io::{self, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    /// Unix Domain Socket Server (equivalent to Windows named pipe)
    pub struct PipeServer {
        listener: UnixListener,
        current_stream: Option<UnixStream>,
        socket_path: String,
    }

    impl PipeServer {
        pub fn create(socket_path: &str) -> io::Result<Self> {
            // Remove existing socket file if present
            let _ = std::fs::remove_file(socket_path);

            let listener = UnixListener::bind(socket_path)?;

            Ok(Self {
                listener,
                current_stream: None,
                socket_path: socket_path.to_string(),
            })
        }

        pub fn pipe_name(&self) -> &str {
            &self.socket_path
        }

        pub fn wait_for_connection(&mut self) -> io::Result<()> {
            let (stream, _) = self.listener.accept()?;
            self.current_stream = Some(stream);
            Ok(())
        }

        pub fn disconnect(&mut self) -> io::Result<()> {
            self.current_stream = None;
            Ok(())
        }

        pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match &mut self.current_stream {
                Some(stream) => stream.read(buf),
                None => Err(io::Error::new(io::ErrorKind::NotConnected, "No client connected")),
            }
        }

        pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match &mut self.current_stream {
                Some(stream) => stream.write(buf),
                None => Err(io::Error::new(io::ErrorKind::NotConnected, "No client connected")),
            }
        }

        pub fn flush(&mut self) -> io::Result<()> {
            match &mut self.current_stream {
                Some(stream) => stream.flush(),
                None => Ok(()),
            }
        }
    }

    impl Drop for PipeServer {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    /// Unix Domain Socket Client
    pub struct PipeClient {
        stream: UnixStream,
    }

    impl PipeClient {
        pub fn connect(socket_path: &str) -> io::Result<Self> {
            let stream = UnixStream::connect(socket_path)?;
            Ok(Self { stream })
        }

        pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.stream.read(buf)
        }

        pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.stream.write(buf)
        }

        pub fn flush(&mut self) -> io::Result<()> {
            self.stream.flush()
        }
    }

    impl Read for PipeClient {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            PipeClient::read(self, buf)
        }
    }

    impl Write for PipeClient {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            PipeClient::write(self, buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            PipeClient::flush(self)
        }
    }

    pub fn pipe_exists(socket_path: &str) -> bool {
        Path::new(socket_path).exists()
    }
}

// Re-export based on platform
#[cfg(windows)]
pub use windows::{PipeServer, PipeClient, pipe_exists};

#[cfg(not(windows))]
pub use unix::{PipeServer, PipeClient, pipe_exists};

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    #[cfg(windows)]
    fn test_pipe_creation() {
        let pipe_name = r"\\.\pipe\test_teamengram_pipe";
        let server = PipeServer::create(pipe_name).expect("Failed to create pipe");
        assert_eq!(server.pipe_name(), pipe_name);
    }

    #[test]
    #[cfg(windows)]
    fn test_pipe_connect() {
        let pipe_name = r"\\.\pipe\test_teamengram_connect";

        // Start server in separate thread
        let server_thread = thread::spawn(move || {
            let server = PipeServer::create(pipe_name).expect("Failed to create pipe");
            server.wait_for_connection().expect("Failed to accept connection");

            // Read message from client
            let mut buf = [0u8; 1024];
            let n = server.read(&mut buf).expect("Failed to read");
            let msg = String::from_utf8_lossy(&buf[..n]);
            assert_eq!(msg, "Hello from client");

            // Send response
            server.write(b"Hello from server").expect("Failed to write");
        });

        // Give server time to start
        thread::sleep(Duration::from_millis(100));

        // Connect as client
        let client = PipeClient::connect(pipe_name).expect("Failed to connect");

        // Send message
        client.write(b"Hello from client").expect("Failed to write");

        // Read response
        let mut buf = [0u8; 1024];
        let n = client.read(&mut buf).expect("Failed to read");
        let msg = String::from_utf8_lossy(&buf[..n]);
        assert_eq!(msg, "Hello from server");

        server_thread.join().expect("Server thread panicked");
    }
}
