//! Real `VolumeLabelResolver`: enumerates the connected drives with
//! `GetLogicalDriveStringsW` and reads each label with `GetVolumeInformationW`.
//!
//! Only fixed and removable drives are asked. A network drive whose server is
//! down, or an optical drive, can block for many seconds on the label query,
//! and a `.wb` carrying `::LABEL` paths means a local or removable disk anyway.

use gb_core::ports::volume_label::{LabelResolution, VolumeLabelResolver};
use gb_core::volume_label::label_key;
use windows_sys::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
};
use windows_sys::Win32::System::Diagnostics::Debug::{SetThreadErrorMode, SEM_FAILCRITICALERRORS};
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};

/// Longest volume label Windows reports (`MAX_PATH + 1` UTF-16 units).
const LABEL_BUFFER_LEN: usize = 261;

pub struct RealVolumeLabelResolver;

impl VolumeLabelResolver for RealVolumeLabelResolver {
    fn resolve(&self, label: &str) -> LabelResolution {
        let _mode = ErrorModeGuard::fail_critical_errors();
        let roots = matching_roots(label);
        match roots.as_slice() {
            [] => LabelResolution::NotConnected,
            [root] => LabelResolution::Resolved(root.clone()),
            _ => LabelResolution::Ambiguous,
        }
    }
}

/// Keeps `SEM_FAILCRITICALERRORS` set on this thread while alive, restoring the
/// previous mode on drop (so also on panic). An empty card-reader slot would
/// otherwise raise a "no disk" dialog on the label query; with the flag the
/// call just fails.
struct ErrorModeGuard {
    previous: Option<u32>,
}

impl ErrorModeGuard {
    fn fail_critical_errors() -> Self {
        let mut previous = 0u32;
        // SAFETY: only changes this thread's error mode; `previous` is a valid
        // out-pointer for the call.
        let changed = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, &mut previous) } != 0;
        Self {
            previous: changed.then_some(previous),
        }
    }
}

impl Drop for ErrorModeGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            // SAFETY: as in `fail_critical_errors`; a null old-mode pointer is allowed.
            unsafe { SetThreadErrorMode(previous, std::ptr::null_mut()) };
        }
    }
}

/// Roots (`E:\`) of the fixed/removable drives whose label equals `label`.
fn matching_roots(label: &str) -> Vec<String> {
    let wanted = label_key(label);
    drive_roots()
        .into_iter()
        .filter(|root| volume_label(root).is_some_and(|actual| label_key(&actual) == wanted))
        .map(|root| String::from_utf16_lossy(&root[..root.len() - 1]))
        .collect()
}

/// Null-terminated UTF-16 roots of every fixed or removable drive.
fn drive_roots() -> Vec<Vec<u16>> {
    // 26 letters * ("X:\" + NUL) + the final NUL.
    let mut buf = [0u16; 26 * 4 + 1];
    // SAFETY: `buf` is valid for `buf.len()` UTF-16 units for the whole call.
    let written = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
    if written == 0 || written > buf.len() {
        return Vec::new();
    }
    buf[..written]
        .split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut root = s.to_vec();
            root.push(0);
            root
        })
        .filter(|root| {
            // SAFETY: `root` is a valid null-terminated UTF-16 string.
            let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
            kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE
        })
        .collect()
}

/// The drive's volume label, or `None` when it cannot be read (no media, not
/// ready, access denied).
fn volume_label(root: &[u16]) -> Option<String> {
    let mut name = [0u16; LABEL_BUFFER_LEN];
    // SAFETY: `root` is a valid null-terminated UTF-16 string and `name` is
    // valid for `LABEL_BUFFER_LEN` units; the unused out-parameters are null,
    // which the API permits.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            name.as_mut_ptr(),
            LABEL_BUFFER_LEN as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    Some(String::from_utf16_lossy(&name[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A label no drive can carry, so this only checks the calls run and
    /// report "not connected" rather than panic or hang.
    #[cfg(windows)]
    #[test]
    fn an_unused_label_is_not_connected() {
        assert_eq!(
            RealVolumeLabelResolver.resolve("GB-NO-SUCH-LABEL-\u{1F4BE}"),
            LabelResolution::NotConnected
        );
    }
}
