use litematica_preview_native::PreviewOptions;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN_BYTES: usize = 32;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

pub struct DecoderProcess {
    child: Child,
    stream: Option<TcpStream>,
    // Retain the child's stdin so its host-lifetime watchdog remains active
    // during native calls.
    input: Option<ChildStdin>,
    memory_limit_mb: Option<u16>,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl DecoderProcess {
    pub fn spawn(memory_limit_mb: Option<u16>) -> Result<Self, String> {
        PreviewOptions {
            memory_limit_mb,
            ..PreviewOptions::default()
        }
        .validate()?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("Unable to create the decoder connection: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("Unable to configure the decoder connection: {e}"))?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("Unable to locate the decoder connection: {e}"))?;
        let token = authentication_token()?;
        let executable = std::env::current_exe()
            .map_err(|e| format!("Unable to locate the decoder executable: {e}"))?;
        let mut command = Command::new(executable);
        command
            .arg("--preview-worker")
            .arg(address.port().to_string())
            .stdin(Stdio::piped())
            // Decoder output carries no protocol data. Null streams prevent native
            // writes from filling a pipe and deadlocking the decoder.
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        #[cfg(windows)]
        let job = decoder_job(memory_limit_mb)?;
        let mut child = command
            .spawn()
            .map_err(|e| format!("Unable to start the decoder process: {e}"))?;
        let input = child.stdin.take();
        let mut process = Self {
            child,
            stream: None,
            input,
            memory_limit_mb,
            #[cfg(windows)]
            _job: job,
        };
        #[cfg(windows)]
        process.assign_job()?;
        process
            .input
            .as_mut()
            .ok_or("The decoder input pipe is unavailable.")?
            .write_all(&token)
            .and_then(|()| {
                process
                    .input
                    .as_mut()
                    .ok_or_else(|| io::Error::other("The decoder input pipe is unavailable"))?
                    .write_all(&memory_limit_mb.unwrap_or(0).to_le_bytes())
            })
            .map_err(|e| process.failure(e))?;

        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if let Some(status) = process
                .child
                .try_wait()
                .map_err(|e| format!("Unable to inspect the decoder process: {e}"))?
            {
                return Err(format!(
                    "The decoder process stopped during startup ({status})."
                ));
            }
            if Instant::now() >= deadline {
                return Err("The decoder process did not connect within ten seconds.".into());
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let timeout = deadline.saturating_duration_since(Instant::now());
                    if timeout.is_zero() {
                        continue;
                    }
                    stream
                        .set_nonblocking(false)
                        .and_then(|()| stream.set_read_timeout(Some(timeout)))
                        .map_err(|e| format!("Unable to configure decoder authentication: {e}"))?;
                    let mut received = [0; TOKEN_BYTES];
                    if stream.read_exact(&mut received).is_err() || received != token {
                        continue;
                    }
                    stream
                        .set_read_timeout(None)
                        .and_then(|()| stream.set_nodelay(true))
                        .map_err(|e| format!("Unable to configure the decoder connection: {e}"))?;
                    process.stream = Some(stream);
                    return Ok(process);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(format!("Unable to connect to the decoder: {error}")),
            }
        }
    }

    pub fn memory_limit_mb(&self) -> Option<u16> {
        self.memory_limit_mb
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Loads one preview request from the isolated decoder.
    ///
    /// An outer error invalidates the decoder process. An inner error preserves
    /// the connection and resource-pack cache for the next request.
    pub fn load(
        &mut self,
        path: &Path,
        pack_path: &Path,
        chunk_size: Option<u16>,
        thread_count: Option<u8>,
        speed_first: bool,
        current: impl Fn() -> bool,
        on_progress: impl FnMut(u64, u64),
    ) -> Result<Result<crate::protocol::Payload, String>, String> {
        self.request(
            path,
            pack_path,
            chunk_size,
            thread_count,
            speed_first,
            |stream| crate::protocol::receive(stream, current, on_progress),
        )
    }

    pub fn load_stream(
        &mut self,
        path: &Path,
        pack_path: &Path,
        chunk_size: Option<u16>,
        thread_count: Option<u8>,
        speed_first: bool,
        current: impl Fn() -> bool,
        on_progress: impl FnMut(u64, u64),
        on_chunk: impl FnMut(usize, crate::protocol::Payload) -> Result<(), String>,
    ) -> Result<Result<crate::protocol::Metadata, String>, String> {
        self.request(
            path,
            pack_path,
            chunk_size,
            thread_count,
            speed_first,
            |stream| crate::protocol::receive_stream(stream, current, on_progress, on_chunk),
        )
    }

    fn request<T>(
        &mut self,
        path: &Path,
        pack_path: &Path,
        chunk_size: Option<u16>,
        thread_count: Option<u8>,
        speed_first: bool,
        receive: impl FnOnce(&mut TcpStream) -> io::Result<Result<T, String>>,
    ) -> Result<Result<T, String>, String> {
        let request = serde_json::to_vec(&(path, pack_path, chunk_size, thread_count, speed_first))
            .map_err(|e| format!("Unable to describe the decoder request: {e}"))?;
        if request.len() > MAX_REQUEST_BYTES {
            return Ok(Err("The schematic or resource path is too long.".into()));
        }
        let result = (|| {
            let stream = self.stream.as_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "The decoder is not connected")
            })?;
            write_frame(stream, &request)?;
            receive(stream)
        })();
        result.map_err(|error| self.failure(error))
    }

    #[cfg(windows)]
    fn assign_job(&self) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        let assigned = unsafe {
            AssignProcessToJobObject(self._job.as_raw_handle(), self.child.as_raw_handle())
        };
        if assigned == 0 {
            return Err(format!(
                "Unable to isolate the decoder process: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    fn failure(&mut self, error: io::Error) -> String {
        // EOF can precede the operating system's exit notification. Allow brief
        // teardown, but bound the wait if the peer remains stuck.
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return format!("The decoder process stopped unexpectedly ({status}). The schematic could not be loaded.{}", process_limit_message(self.memory_limit_mb));
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => return format!("The decoder connection failed: {error}"),
            }
        }
    }
}

impl Drop for DecoderProcess {
    fn drop(&mut self) {
        // Closing stdin makes the child watchdog terminate the process, including
        // during native calls. kill/wait prevents zombies during ordinary replacement.
        self.input.take();
        self.stream.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn preview_memory(decoder_pid: u32) -> Option<u64> {
    let host = private_working_set(std::process::id())?;
    let decoder = private_working_set(decoder_pid)?;
    host.checked_add(decoder)
}

#[cfg(windows)]
fn private_working_set(pid: u32) -> Option<u64> {
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const SYNCHRONIZE: u32 = 0x0010_0000;

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    if unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } != WAIT_TIMEOUT {
        return None;
    }
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            handle.as_raw_handle(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast(),
            size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        )
    };
    (ok != 0 && unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } == WAIT_TIMEOUT)
        .then_some(counters.PrivateWorkingSetSize as u64)
}

#[cfg(not(windows))]
fn private_working_set(_: u32) -> Option<u64> {
    None
}
pub fn run(port: &std::ffi::OsStr) -> Result<(), String> {
    let port: u16 = port
        .to_str()
        .and_then(|value| value.parse().ok())
        .filter(|port| *port != 0)
        .ok_or("Invalid decoder connection port.")?;
    let mut token = [0; TOKEN_BYTES];
    io::stdin()
        .read_exact(&mut token)
        .map_err(|e| format!("Unable to read decoder authentication: {e}"))?;
    // Accept the launch memory cap only from the inherited host pipe, never from
    // a request packet.
    let mut cap = [0; 2];
    io::stdin()
        .read_exact(&mut cap)
        .map_err(|e| format!("Unable to read decoder memory settings: {e}"))?;
    let memory_limit_mb = (u16::from_le_bytes(cap) != 0).then_some(u16::from_le_bytes(cap));
    PreviewOptions {
        memory_limit_mb,
        ..PreviewOptions::default()
    }
    .validate()?;
    // The child may be stuck in native code that cannot unwind. Its watchdog exits
    // when the host closes stdin, preventing an orphaned decoder.
    std::thread::Builder::new()
        .name("preview-host-lifetime".into())
        .spawn(|| {
            let mut byte = [0];
            let mut input = io::stdin().lock();
            loop {
                match input.read(&mut byte) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    _ => std::process::exit(0),
                }
            }
        })
        .map_err(|e| format!("Unable to monitor the preview host: {e}"))?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, STARTUP_TIMEOUT)
        .map_err(|e| format!("Unable to connect to the preview host: {e}"))?;
    stream
        .set_nodelay(true)
        .and_then(|()| stream.write_all(&token))
        .map_err(|e| format!("Unable to authenticate the decoder: {e}"))?;
    let mut pack = None;
    loop {
        let request = read_frame(&mut stream, MAX_REQUEST_BYTES)
            .map_err(|e| format!("Unable to read the decoder request: {e}"))?;
        let (path, pack_path, chunk_size, thread_count, speed_first): (
            PathBuf,
            PathBuf,
            Option<u16>,
            Option<u8>,
            bool,
        ) = serde_json::from_slice(&request)
            .map_err(|e| format!("Invalid decoder request: {e}"))?;
        let options = PreviewOptions {
            memory_limit_mb,
            chunk_size,
            thread_count,
            speed_first,
        };
        let result = crate::preview::decode(&path, &pack_path, &mut pack, options, &mut stream);
        crate::protocol::finish(&mut stream, result)
            .map_err(|e| format!("Unable to send the decoder response: {e}"))?;
    }
}

fn read_frame(stream: &mut TcpStream, maximum: usize) -> io::Result<Vec<u8>> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > maximum {
        return Err(invalid_data("The decoder packet length is invalid"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| io::Error::other("There is not enough memory to receive this preview"))?;
    bytes.resize(length, 0);
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> io::Result<()> {
    let length =
        u32::try_from(bytes.len()).map_err(|_| invalid_data("The decoder packet is too large"))?;
    stream.write_all(&length.to_le_bytes())?;
    stream.write_all(bytes)
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(windows)]
fn authentication_token() -> Result<[u8; TOKEN_BYTES], String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut token = [0; TOKEN_BYTES];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            token.as_mut_ptr(),
            token.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(format!(
            "Unable to authenticate the decoder process (OS status {status})."
        ));
    }
    Ok(token)
}

#[cfg(not(windows))]
fn authentication_token() -> Result<[u8; TOKEN_BYTES], String> {
    let mut token = [0; TOKEN_BYTES];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut token))
        .map_err(|e| format!("Unable to authenticate the decoder process: {e}"))?;
    Ok(token)
}

#[cfg(windows)]
fn decoder_job(memory_limit_mb: Option<u16>) -> Result<std::os::windows::io::OwnedHandle, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };
    // Keep the job handle non-inheritable so host termination closes the last handle
    // and triggers JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE. Refuse to decode if setup fails.
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(format!(
            "Unable to create the decoder job: {}",
            io::Error::last_os_error()
        ));
    }
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if let Some(mb) = memory_limit_mb {
        limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.ProcessMemoryLimit = usize::from(mb)
            .checked_mul(1024 * 1024)
            .ok_or("The decoder memory limit exceeds this platform's addressable memory.")?;
    }
    let configured = unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    };
    if configured == 0 {
        return Err(format!(
            "Unable to configure decoder isolation: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(job)
}

fn process_limit_message(memory_limit_mb: Option<u16>) -> String {
    match memory_limit_mb {
        Some(mb) if cfg!(windows) => {
            format!(" Decoder memory was limited to {mb} MB; the exit alone cannot confirm whether that limit was reached.")
        }
        None => " The decoder memory limit is disabled.".into(),
        Some(_) => String::new(),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::decoder_job;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        JobObjectExtendedLimitInformation, QueryInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };

    #[test]
    fn private_working_set_reads_live_process_and_rejects_missing_process() {
        assert!(super::private_working_set(std::process::id()).is_some_and(|bytes| bytes > 0));
        assert_eq!(super::private_working_set(0), None);
    }

    #[test]
    fn changing_or_disabling_memory_cap_retains_kill_on_close_isolation() {
        for cap in [None, Some(2048), Some(3073), Some(8192), None] {
            let job = decoder_job(cap).unwrap();
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            let queried = unsafe {
                QueryInformationJobObject(
                    job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(queried, 0);
            assert_ne!(
                limits.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                0
            );
            assert_eq!(
                limits.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_PROCESS_MEMORY != 0,
                cap.is_some()
            );
            assert_eq!(
                limits.ProcessMemoryLimit,
                usize::from(cap.unwrap_or(0)) * 1024 * 1024
            );
        }
    }
}
