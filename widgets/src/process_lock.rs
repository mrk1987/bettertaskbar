//! A named Windows mutex serializes read/modify/rename across widget processes.
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

// Ownership is thread-affine: ReleaseMutex must run on the acquiring thread.
pub struct Lock(HANDLE, std::marker::PhantomData<std::rc::Rc<()>>);

impl Lock {
    pub fn acquire(name: &str) -> std::io::Result<Self> {
        let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }
            .map_err(std::io::Error::other)?;
        let result = unsafe { WaitForSingleObject(handle, 5_000) };
        if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
            Ok(Self(handle, std::marker::PhantomData))
        } else {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Err(std::io::Error::other(format!(
                "cannot acquire config lock: {result:?}"
            )))
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn serializes_read_modify_write() {
        let value = Arc::new(AtomicUsize::new(0));
        let name = format!("Local\\BetterTaskbarLockTest-{}", std::process::id());
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (value, name) = (value.clone(), name.clone());
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        let _lock = Lock::acquire(&name).unwrap();
                        let old = value.load(Ordering::Relaxed);
                        std::thread::yield_now();
                        value.store(old + 1, Ordering::Relaxed);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(value.load(Ordering::Relaxed), 100);
    }
}
