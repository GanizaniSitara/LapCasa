//! Open the Windows default mail client's composer, with no sender or recipient.
#[cfg(target_os = "windows")]
mod windows_mail {
    use std::{ffi::c_void, path::Path, ptr::null_mut};
    use windows::{core::{s, w}, Win32::{Foundation::{FreeLibrary, HMODULE}, System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
    }}};

    // Simple MAPI Unicode ABI, documented in Windows SDK mapi.h. Attachments
    // remain alive until the modal composer returns and the client has copied them.
    #[repr(C)]
    struct File { reserved: u32, flags: u32, position: u32, path: *mut u16, name: *mut u16, file_type: *mut c_void }
    #[repr(C)]
    struct Message {
        reserved: u32, subject: *mut u16, body: *mut u16, message_type: *mut u16,
        date: *mut u16, conversation: *mut u16, flags: u32, originator: *mut c_void,
        recipient_count: u32, recipients: *mut c_void, file_count: u32, files: *mut File,
    }
    type SendMail = unsafe extern "system" fn(usize, usize, *mut Message, u32, u32) -> u32;
    // MAPI_DIALOG is mandatory: this application never requests silent sending.
    const COMPOSE_FLAGS: u32 = 0x8 | 0x1 | 0x40000; // DIALOG | LOGON_UI | FORCE_UNICODE

    fn wide(text: &str) -> Result<Vec<u16>, String> {
        if text.contains('\0') { return Err("Invalid email attachment name".into()); }
        Ok(text.encode_utf16().chain(Some(0)).collect())
    }

    fn compose_with(paths: &[String], send: SendMail) -> Result<bool, String> {
        if paths.is_empty() { return Err("No files to email".into()); }
        let count = u32::try_from(paths.len()).map_err(|_| "Too many email attachments")?;
        let mut names: Vec<Vec<u16>> = paths.iter().map(|p| wide(&Path::new(p).file_name()
            .ok_or("Invalid attachment path")?.to_string_lossy())).collect::<Result<_, _>>()?;
        let mut paths = paths.iter().map(|p| wide(p)).collect::<Result<Vec<_>, _>>()?;
        let mut files: Vec<File> = paths.iter_mut().zip(names.iter_mut()).map(|(path, name)| File {
            reserved: 0, flags: 0, position: u32::MAX, path: path.as_mut_ptr(), name: name.as_mut_ptr(), file_type: null_mut(),
        }).collect();
        let mut subject = wide(&format!("Photos from LapCasa ({count})"))?;
        let mut body = wide("Attached photos from LapCasa.")?;
        let mut message = Message { reserved: 0, subject: subject.as_mut_ptr(), body: body.as_mut_ptr(),
            message_type: null_mut(), date: null_mut(), conversation: null_mut(), flags: 0,
            originator: null_mut(), recipient_count: 0, recipients: null_mut(), file_count: count, files: files.as_mut_ptr() };
        // No sender, recipient or credentials are supplied. The user reviews the
        // default mail client's dialog and chooses whether to send there.
        match unsafe { send(0, 0, &mut message, COMPOSE_FLAGS, 0) } {
            0 => Ok(true),
            1 => Ok(false), // User cancelled; do not show a success or error toast.
            code => Err(format!("Your email app could not open photo attachments (Windows code {code}). Export the photos and attach them in your email app.")),
        }
    }

    pub fn compose(paths: &[String]) -> Result<bool, String> {
        struct Library(HMODULE);
        impl Drop for Library { fn drop(&mut self) { unsafe { let _ = FreeLibrary(self.0); } } }
        struct Apartment;
        impl Drop for Apartment { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().map_err(|e| e.to_string())?;
            let _apartment = Apartment;
            let library = Library(LoadLibraryExW(w!("mapi32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
                .map_err(|_| "A compatible default Windows email app is required to compose attachments")?);
            let address = GetProcAddress(library.0, s!("MAPISendMailW"))
                .ok_or("Your default email app does not support Unicode photo attachments")?;
            let send: SendMail = std::mem::transmute(address);
            compose_with(paths, send)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        unsafe extern "system" fn check_message(session: usize, owner: usize, message: *mut Message, flags: u32, reserved: u32) -> u32 {
            let m = unsafe { &*message };
            assert_eq!((session, owner, reserved), (0, 0, 0));
            assert_eq!(flags & 0x8, 0x8);
            assert!(m.originator.is_null() && m.recipients.is_null());
            assert_eq!(m.recipient_count, 0);
            assert_eq!(m.file_count, 1);
            let f = unsafe { &*m.files };
            assert_eq!(f.position, u32::MAX);
            assert_eq!(unsafe { *f.name }, '旅' as u16);
            1
        }
        #[test]
        fn composer_always_requires_review_and_leaves_recipients_empty() {
            assert!(!compose_with(&["C:/photos/旅行.jpg".into()], check_message).unwrap());
            assert!(compose_with(&[], check_message).is_err());
            assert!(compose_with(&["C:/photos/bad\0.jpg".into()], check_message).is_err());
        }
    }
}

pub fn compose(paths: &[String]) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    { windows_mail::compose(paths) }
    #[cfg(not(target_os = "windows"))]
    { let _ = paths; Err("Export the selected photos and attach them in your email app".into()) }
}
