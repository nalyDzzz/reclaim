//! Windows' own Disk Cleanup handlers: the COM objects that Settings >
//! Storage > Temporary files and Disk Cleanup run for items like "Previous
//! Windows installation(s)". Reclaim asks a handler how much it can free and
//! then has it free that, so Windows' own code decides what is safe to
//! remove and how. That covers what a plain folder delete would get wrong,
//! like TrustedInstaller-owned files and the state behind "Go back".

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use windows::Win32::Foundation::S_FALSE;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize, IClassFactory};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::Win32::System::Registry::HKEY;
use windows::Win32::UI::LegacyWindowsEnvironmentFeatures::{
    EMPTY_VOLUME_CACHE_FLAGS, IEmptyVolumeCache, IEmptyVolumeCache2, IEmptyVolumeCacheCallBack, IEmptyVolumeCacheCallBack_Impl,
};
use windows::core::{GUID, HRESULT, HSTRING, Interface, PCWSTR, PWSTR, implement, s};
use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};

const VOLUME_CACHES: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VolumeCaches";

/// How long a scan may take before Reclaim stops waiting for it.
const SCAN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Handler {
    /// Its key under VolumeCaches, which names its class and holds its settings.
    pub key: &'static str,
    /// The System32 DLL that implements it.
    pub dll: &'static str,
}

/// "Previous Windows installation(s)": Windows.old.
pub const PREVIOUS_INSTALLATIONS: Handler = Handler { key: "Previous Installations", dll: "setupcln.dll" };

/// Handlers run one at a time. Two at once could have Windows' servicing
/// stack check and clean up the same files together, and a scan that timed
/// out may still be running when a cleanup starts.
static BUSY: Mutex<()> = Mutex::new(());

/// How much the handler can free on the system drive.
pub fn measure(handler: Handler, elevated: bool) -> anyhow::Result<u64> {
    on_com_thread(Some(SCAN_TIMEOUT), move || {
        let _busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        let session = Session::open(handler, elevated)?;
        if session.empty { Ok(0) } else { session.space_used() }
    })
}

/// Has the handler free everything it found. `progress` gets the bytes
/// freed so far and the total, as often as the handler reports them.
pub fn purge(handler: Handler, elevated: bool, progress: impl Fn(u64, u64) + Send + 'static) -> anyhow::Result<()> {
    on_com_thread(None, move || {
        let _busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        let session = Session::open(handler, elevated)?;
        if session.empty {
            return Ok(());
        }
        // Handlers free what their scan found, so they always scan first.
        let space = session.space_used()?;
        if space == 0 {
            return Ok(());
        }
        let callback: IEmptyVolumeCacheCallBack = Callback { on_purge: Some(Box::new(progress)) }.into();
        let cache = &session.cache;
        unsafe { (cache.vtable().Purge)(cache.as_raw(), space, callback.as_raw()) }.ok().context("Windows couldn't finish the cleanup")?;
        Ok(())
    })
}

struct Session {
    cache: IEmptyVolumeCache,
    /// The handler said it has nothing to clean.
    empty: bool,
    /// The handler's registry key, which it may use until it's deactivated.
    _key: RegKey,
}

impl Session {
    /// Creates and initializes the handler.
    fn open(handler: Handler, elevated: bool) -> anyhow::Result<Session> {
        let path = format!(r"{VOLUME_CACHES}\{}", handler.key);
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        // Handlers may record state in their key, as they do under Disk Cleanup.
        let key = elevated
            .then(|| hklm.open_subkey_with_flags(&path, KEY_READ | KEY_WRITE).ok())
            .flatten()
            .map_or_else(|| hklm.open_subkey_with_flags(&path, KEY_READ), Ok)
            .context("This version of Windows doesn't have this cleanup")?;
        // Only administrators can change HKLM, so the class it names is trusted.
        let clsid: String = key.get_value("").context("Windows' cleanup for this isn't registered")?;
        let clsid = GUID::try_from(clsid.trim().trim_start_matches('{').trim_end_matches('}'))
            .map_err(|_| anyhow!("Windows' cleanup for this is registered under an unexpected name"))?;
        let cache = create(handler.dll, &clsid)?;

        let hkey = HKEY(key.raw_handle());
        // These handlers work on the system drive.
        let volume = HSTRING::from(super::system_root().as_os_str());
        let key_name = HSTRING::from(handler.key);
        // Never EVCF_SETTINGSMODE: in that mode handlers delete right away,
        // inside InitializeEx, instead of waiting to be asked.
        let mut flags = EMPTY_VOLUME_CACHE_FLAGS(0);
        let (mut name, mut description, mut button) = (PWSTR::null(), PWSTR::null(), PWSTR::null());
        let hr: HRESULT = unsafe {
            match cache.cast::<IEmptyVolumeCache2>() {
                Ok(v2) => (v2.vtable().InitializeEx)(
                    v2.as_raw(),
                    hkey,
                    PCWSTR(volume.as_ptr()),
                    PCWSTR(key_name.as_ptr()),
                    &mut name,
                    &mut description,
                    &mut button,
                    &mut flags,
                ),
                Err(_) => {
                    (cache.vtable().Initialize)(cache.as_raw(), hkey, PCWSTR(volume.as_ptr()), &mut name, &mut description, &mut flags)
                }
            }
        };
        for text in [name, description, button] {
            if !text.is_null() {
                unsafe { CoTaskMemFree(Some(text.0 as *const c_void)) };
            }
        }
        hr.ok().context("Windows' cleanup for this didn't start")?;
        // S_FALSE: "There are no files to delete." It's still deactivated
        // once done with, as Disk Cleanup does.
        Ok(Session { cache, empty: hr == S_FALSE, _key: key })
    }

    fn space_used(&self) -> anyhow::Result<u64> {
        let callback: IEmptyVolumeCacheCallBack = Callback { on_purge: None }.into();
        let mut bytes = 0u64;
        let hr = unsafe { (self.cache.vtable().GetSpaceUsed)(self.cache.as_raw(), &mut bytes, callback.as_raw()) };
        hr.ok().context("Windows couldn't check how much this frees")?;
        // S_FALSE means the count failed, and -1 that the handler can't tell.
        if hr == S_FALSE || bytes == u64::MAX {
            bail!("Windows couldn't work out how much this frees");
        }
        Ok(bytes)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // The flags it returns can ask Disk Cleanup to unregister the handler.
        // That's Disk Cleanup's business, so Reclaim ignores them.
        let _ = unsafe { self.cache.Deactivate() };
    }
}

type GetClassObject = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

/// Creates the handler from its DLL in System32. Going through
/// CoCreateInstance would look the class up in HKEY_CLASSES_ROOT, which
/// includes the user's own registrations. A standard user could point those
/// at any DLL and have an elevated Reclaim load it.
fn create(dll: &str, clsid: &GUID) -> anyhow::Result<IEmptyVolumeCache> {
    unsafe {
        // Stays loaded: the handler's objects live in it.
        let module = LoadLibraryExW(&HSTRING::from(dll), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
            .with_context(|| format!("Couldn't load Windows' {dll}"))?;
        let get = GetProcAddress(module, s!("DllGetClassObject")).ok_or_else(|| anyhow!("{dll} isn't a cleanup handler"))?;
        let get: GetClassObject = std::mem::transmute(get);
        let mut factory = std::ptr::null_mut();
        get(clsid, &IClassFactory::IID, &mut factory).ok().context("Windows' cleanup for this has moved")?;
        let factory = IClassFactory::from_raw(factory);
        Ok(factory.CreateInstance::<_, IEmptyVolumeCache>(None)?)
    }
}

/// Runs `work` on a thread of its own with COM set up the way Disk Cleanup
/// has it. Gives up waiting after `timeout`, leaving the thread to finish.
fn on_com_thread<T: Send + 'static>(
    timeout: Option<Duration>,
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new().name("cleanup-handler".into()).spawn(move || {
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // Every COM object `work` made is gone by the time it returns.
        let result = work();
        if com.is_ok() {
            unsafe { CoUninitialize() };
        }
        let _ = tx.send(result);
    })?;
    match timeout {
        Some(t) => rx.recv_timeout(t).map_err(|_| anyhow!("Windows took too long to check this"))?,
        None => rx.recv().map_err(|_| anyhow!("Windows' cleanup stopped unexpectedly"))?,
    }
}

#[implement(IEmptyVolumeCacheCallBack)]
struct Callback {
    on_purge: Option<Box<dyn Fn(u64, u64) + Send>>,
}

impl IEmptyVolumeCacheCallBack_Impl for Callback_Impl {
    fn ScanProgress(&self, _used: u64, _flags: u32, _status: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn PurgeProgress(&self, freed: u64, to_free: u64, _flags: u32, _status: &PCWSTR) -> windows::core::Result<()> {
        if let Some(report) = &self.on_purge {
            report(freed, to_free);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scans only: GetSpaceUsed is what Disk Cleanup runs when it opens.
    #[test]
    #[ignore = "asks Windows' cleanup handlers to scan this PC"]
    fn scans_this_pc() {
        let elevated = crate::system::is_elevated();
        let temp_setup = Handler { key: "Temporary Setup Files", dll: "setupcln.dll" };
        for handler in [PREVIOUS_INSTALLATIONS, temp_setup] {
            println!("{} (elevated {elevated}): {:?}", handler.key, measure(handler, elevated));
        }
    }
}
