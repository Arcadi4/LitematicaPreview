use std::cell::RefCell;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use litematica_preview_core::{MeshBatch, PreviewOptions};
use parking_lot::Mutex;

use crate::preview::{Metadata, PreviewWorker};
use crate::preview_protocol::{self as wire, Event};

const TOKEN_BYTES: usize = 32;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(windows)]
const DECODER_MEMORY_BYTES: usize = 2 * 1024 * 1024 * 1024;

struct ProcessState {
    child: Option<Child>,
    input: Option<ChildStdin>,
    stream: Option<TcpStream>,
}

pub(crate) struct DecoderControl {
    stopped: AtomicBool,
    state: Mutex<ProcessState>,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl DecoderControl {
    pub(crate) fn invalidate(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    /// Interrupts native work and IPC without waiting for process teardown.
    pub(crate) fn stop(&self) {
        self.invalidate();
        let (input, stream) = {
            let mut state = self.state.lock();
            if let Some(child) = state.child.as_mut() {
                let _ = child.kill();
            }
            (state.input.take(), state.stream.take())
        };
        drop(input);
        if let Some(stream) = stream {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn connect(&self, stream: &TcpStream) -> io::Result<()> {
        let stream = stream.try_clone()?;
        let mut state = self.state.lock();
        if self.stopped.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
        }
        state.stream = Some(stream);
        Ok(())
    }

    fn exited(&self) -> Result<Option<std::process::ExitStatus>, String> {
        self.state
            .lock()
            .child
            .as_mut()
            .ok_or_else(|| "The decoder process is unavailable.".to_string())?
            .try_wait()
            .map_err(|e| format!("Unable to inspect the decoder process: {e}"))
    }

    #[cfg(windows)]
    fn memory(&self) -> Option<u64> {
        use std::os::windows::io::AsRawHandle;
        let state = self.state.lock();
        let child = state.child.as_ref()?;
        private_working_set(child.as_raw_handle())
    }
}

pub struct DecoderProcess {
    control: Arc<DecoderControl>,
    stream: Option<BufReader<TcpStream>>,
}

impl DecoderProcess {
    fn spawn(
        register: impl FnOnce(Arc<DecoderControl>) -> Result<(), String>,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("Unable to create the decoder connection: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = authentication_token()?;
        let executable = std::env::current_exe()
            .map_err(|e| format!("Unable to locate the decoder executable: {e}"))?;
        let mut command = Command::new(executable);
        command
            .arg("--preview-worker")
            .arg(port.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        #[cfg(windows)]
        let job = decoder_job()?;
        let mut child = command
            .spawn()
            .map_err(|e| format!("Unable to start the decoder process: {e}"))?;
        let input = child.stdin.take();
        let mut process = Self {
            control: Arc::new(DecoderControl {
                stopped: AtomicBool::new(false),
                state: Mutex::new(ProcessState {
                    child: Some(child),
                    input,
                    stream: None,
                }),
                #[cfg(windows)]
                _job: job,
            }),
            stream: None,
        };
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
            let state = process.control.state.lock();
            let child = state
                .child
                .as_ref()
                .ok_or("The decoder process is unavailable.")?;
            if unsafe {
                AssignProcessToJobObject(
                    process.control._job.as_raw_handle(),
                    child.as_raw_handle(),
                )
            } == 0
            {
                return Err(format!(
                    "Unable to isolate the decoder process: {}",
                    io::Error::last_os_error()
                ));
            }
        }
        register(Arc::clone(&process.control))?;
        // The bounded authentication write fits in the empty inherited pipe.
        let mut input = process
            .control
            .state
            .lock()
            .input
            .take()
            .ok_or("The decoder input pipe is unavailable.")?;
        let authenticated = input.write_all(&token);
        {
            let mut state = process.control.state.lock();
            if !process.control.stopped.load(Ordering::Acquire) {
                state.input = Some(input);
            }
        }
        authenticated.map_err(|e| format!("Unable to authenticate the decoder: {e}"))?;
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if process.control.stopped.load(Ordering::Acquire) {
                return Err("Cancelled".into());
            }
            if let Some(status) = process.control.exited()? {
                return Err(format!(
                    "The decoder process stopped during startup ({status})."
                ));
            }
            if Instant::now() >= deadline {
                return Err("The decoder process did not connect within ten seconds.".into());
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // Darwin inherits the listener's nonblocking mode. Reads
                    // must block between streamed frames until cancellation
                    // shuts the owned socket down.
                    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                    process
                        .control
                        .connect(&stream)
                        .map_err(|e| e.to_string())?;
                    stream
                        .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
                        .map_err(|e| e.to_string())?;
                    let mut received = [0; TOKEN_BYTES];
                    if stream.read_exact(&mut received).is_err() || received != token {
                        continue;
                    }
                    stream
                        .set_read_timeout(None)
                        .and_then(|()| stream.set_nodelay(true))
                        .map_err(|e| e.to_string())?;
                    process.stream = Some(BufReader::new(stream));
                    return Ok(process);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(format!("Unable to connect to the decoder: {error}")),
            }
        }
    }

    fn load(
        &mut self,
        path: &Path,
        pack_path: &Path,
        custom_pack_path: Option<&Path>,
        options: PreviewOptions,
        current: impl Fn() -> Result<(), String>,
        prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
        mut consume: impl FnMut(MeshBatch) -> Result<(), String>,
        mut progress: impl FnMut(&'static str, u64, u64),
    ) -> Result<Result<Metadata, String>, String> {
        let request = serde_json::to_vec(&(
            path,
            pack_path,
            custom_pack_path,
            options.chunk_size,
            options.thread_count,
        ))
        .map_err(|e| format!("Unable to describe the decoder request: {e}"))?;
        let control = &self.control;
        let stream = self
            .stream
            .as_mut()
            .ok_or("The decoder is not connected.")?;
        wire::write_frame(stream.get_mut(), &request, MAX_REQUEST_BYTES)
            .map_err(|e| format!("Unable to send the decoder request: {e}"))?;
        let mut prepared = Some(prepared);
        loop {
            current()?;
            let event = wire::receive(&mut *stream).map_err(|error| {
                let detail = if cfg!(windows) {
                    " Decoder private memory is capped at 2 GiB; an unexpected exit alone does not establish that the cap was reached."
                } else { "" };
                match control.exited() {
                    Ok(Some(status)) => format!("The decoder process or connection failed ({status}): {error}.{detail}"),
                    _ => format!("The decoder process or connection failed: {error}.{detail}"),
                }
            })?;
            current()?;
            match event {
                Event::Progress(phase, completed, total) => progress(phase, completed, total),
                Event::Prepared(min, max) => {
                    prepared.take().ok_or("Duplicate decoder source bounds.")?(min, max)?
                }
                Event::Batch(batch) => {
                    if prepared.is_some() {
                        return Err("The decoder sent geometry before source bounds.".into());
                    }
                    consume(batch)?;
                }
                Event::Finished(result) => return Ok(result),
            }
        }
    }
}

impl Drop for DecoderProcess {
    fn drop(&mut self) {
        self.control.stop();
        self.stream.take();
        let child = self.control.state.lock().child.take();
        if let Some(mut child) = child {
            let _ = child.wait();
        }
    }
}

pub fn load(
    decoder: &mut Option<DecoderProcess>,
    worker: &PreviewWorker,
    id: u64,
    path: &Path,
    pack_path: &Path,
    custom_pack_path: Option<&Path>,
    options: PreviewOptions,
    prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
    consume: impl FnMut(MeshBatch) -> Result<(), String>,
    progress: impl FnMut(&'static str, u64, u64),
) -> Result<Metadata, String> {
    worker.ensure_current(id)?;
    if decoder
        .as_ref()
        .is_some_and(|process| process.control.stopped.load(Ordering::Acquire))
    {
        *decoder = None;
    }
    if decoder.is_none() {
        *decoder = Some(DecoderProcess::spawn(|control| {
            worker.register_decoder(id, control)
        })?);
    }
    let process = decoder
        .as_mut()
        .ok_or("The decoder process is unavailable.")?;
    worker.register_decoder(id, Arc::clone(&process.control))?;
    let result = process.load(
        path,
        pack_path,
        custom_pack_path,
        options,
        || worker.ensure_current(id),
        prepared,
        consume,
        progress,
    );
    match result {
        Ok(result) => result,
        Err(error) => {
            *decoder = None;
            Err(error)
        }
    }
}

pub fn run(port: &std::ffi::OsStr) -> Result<(), String> {
    let port = port
        .to_str()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .ok_or("Invalid decoder connection port.")?;
    let mut token = [0; TOKEN_BYTES];
    io::stdin()
        .read_exact(&mut token)
        .map_err(|e| format!("Unable to read decoder authentication: {e}"))?;
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
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        STARTUP_TIMEOUT,
    )
    .map_err(|e| format!("Unable to connect to the preview host: {e}"))?;
    stream
        .set_nodelay(true)
        .and_then(|()| stream.write_all(&token))
        .map_err(|e| e.to_string())?;
    let mut input = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let output = RefCell::new(BufWriter::new(stream));
    let mut packs = crate::preview::PackCache::default();
    loop {
        let request = wire::read_frame(&mut input, MAX_REQUEST_BYTES).map_err(|e| e.to_string())?;
        let (path, pack_path, custom_pack_path, chunk_size, thread_count): (
            PathBuf,
            PathBuf,
            Option<PathBuf>,
            Option<u16>,
            Option<u8>,
        ) = serde_json::from_slice(&request)
            .map_err(|e| format!("Invalid decoder request: {e}"))?;
        let options = PreviewOptions {
            memory_limit_mb: None,
            chunk_size,
            thread_count,
            speed_first: thread_count.is_some(),
        };
        let transport_error = RefCell::new(None);
        let result = crate::preview::decode(
            &path,
            &pack_path,
            custom_pack_path.as_deref(),
            &mut packs,
            options,
            || match transport_error.borrow().as_ref() {
                Some(error) => Err(format!("The decoder connection failed: {error}")),
                None => Ok(()),
            },
            |min, max| {
                wire::prepared(&mut *output.borrow_mut(), min, max).map_err(|e| e.to_string())
            },
            |batch| wire::batch(&mut *output.borrow_mut(), &batch).map_err(|e| e.to_string()),
            |phase, completed, total| {
                if let Err(error) =
                    wire::progress(&mut *output.borrow_mut(), phase, completed, total)
                {
                    *transport_error.borrow_mut() = Some(error);
                }
            },
        );
        wire::finish(&mut *output.borrow_mut(), result)
            .map_err(|e| format!("Unable to send the decoder result: {e}"))?;
    }
}

pub(crate) fn preview_memory(control: Option<&DecoderControl>) -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let host = private_working_set(unsafe { GetCurrentProcess() })?;
        let decoder = match control {
            Some(control) => control.memory()?,
            None => 0,
        };
        host.checked_add(decoder)
    }
    #[cfg(not(windows))]
    {
        let _ = control;
        None
    }
}

#[cfg(windows)]
fn private_working_set(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            handle,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast(),
            std::mem::size_of_val(&counters) as u32,
        )
    };
    (ok != 0).then_some(counters.PrivateWorkingSetSize as u64)
}

#[cfg(windows)]
fn decoder_job() -> Result<std::os::windows::io::OwnedHandle, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(format!(
            "Unable to create the decoder job: {}",
            io::Error::last_os_error()
        ));
    }
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.ProcessMemoryLimit = DECODER_MEMORY_BYTES;
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(format!(
            "Unable to configure decoder isolation: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(job)
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn stopping_a_busy_process_interrupts_ipc_without_waiting_for_its_work() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut child = Command::new("/bin/sleep")
            .arg("60")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let control = Arc::new(DecoderControl {
            stopped: AtomicBool::new(false),
            state: Mutex::new(ProcessState {
                child: Some(child),
                input,
                stream: Some(stream.try_clone().unwrap()),
            }),
        });
        let process = DecoderProcess {
            control: Arc::clone(&control),
            stream: None,
        };
        let (started_tx, started) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut stream = stream;
            started_tx.send(()).unwrap();
            let error = stream.read_exact(&mut [0]).unwrap_err();
            done_tx.send(error.kind()).unwrap();
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        control.stop();
        let error = done.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            error,
            io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::NotConnected
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = control.exited().unwrap() {
                assert!(!status.success());
                break;
            }
            assert!(Instant::now() < deadline, "the decoder was not killed");
            std::thread::sleep(Duration::from_millis(1));
        }
        reader.join().unwrap();
        drop(process);
        drop(peer);
    }
}
