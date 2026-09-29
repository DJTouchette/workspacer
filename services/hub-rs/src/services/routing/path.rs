use std::path::Path;
#[cfg(not(windows))]
pub(crate) fn path_within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}
#[cfg(windows)]
pub(crate) fn path_within(path: &Path, root: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CompareStringOrdinal(
            a: *const u16,
            an: i32,
            b: *const u16,
            bn: i32,
            ignore_case: i32,
        ) -> i32;
    }
    let mut components = path.components();
    root.components().all(|component| {
        let Some(actual) = components.next() else {
            return false;
        };
        let a: Vec<u16> = actual.as_os_str().encode_wide().collect();
        let b: Vec<u16> = component.as_os_str().encode_wide().collect();
        let an = i32::try_from(a.len()).expect("path component too long");
        let bn = i32::try_from(b.len()).expect("path component too long");
        let comparison = unsafe { CompareStringOrdinal(a.as_ptr(), an, b.as_ptr(), bn, 1) };
        assert_ne!(comparison, 0, "Windows ordinal path comparison failed");
        comparison == 2
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn windows_ordinal_path_boundaries() {
        for (path, root, expected) in [
            (r"C:\Work\Client", r"c:\work\client", true),
            (r"C:\Работа\ПРОЕКТ", r"c:\работа\проект", true),
            (r"C:\Work\Ärger", r"c:\work\ärGER", true),
            (r"C:\Work\Kelvin", r"c:\work\kelvin", false),
            (r"C:\Work\Client-old", r"c:\work\client", false),
            (r"D:\Work\Client", r"c:\work\client", false),
            (r"\\server\other\work", r"\\server\share\work", false),
        ] {
            assert_eq!(
                path_within(Path::new(path), Path::new(root)),
                expected,
                "{path} under {root}"
            );
        }
    }
}
