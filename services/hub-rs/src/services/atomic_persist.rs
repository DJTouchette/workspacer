//! A failed atomic replacement retains both the old document and our staged
//! file. Windows readers can temporarily prevent MoveFileEx replacement.
use std::{io, path::Path, time::Duration};
use tempfile::{NamedTempFile, PersistError};

pub(super) fn persist(file: NamedTempFile, path: &Path) -> io::Result<()> {
    let retries = if cfg!(windows) { 20 } else { 0 };
    retry(
        file,
        retries,
        |file| file.persist(path).map(|_| ()),
        || {
            std::thread::sleep(Duration::from_millis(5));
        },
    )
}

fn retry(
    mut file: NamedTempFile,
    retries: usize,
    mut attempt: impl FnMut(NamedTempFile) -> Result<(), PersistError>,
    mut pause: impl FnMut(),
) -> io::Result<()> {
    for index in 0..=retries {
        match attempt(file) {
            Ok(()) => return Ok(()),
            Err(failure)
                if index < retries && matches!(failure.error.raw_os_error(), Some(5 | 32 | 33)) =>
            {
                // Recover the SAME owned staging file. Never remove the old
                // destination, recreate it in place, or downgrade to copying.
                file = failure.file;
                pause();
            }
            Err(failure) => return Err(failure.error),
        }
    }
    unreachable!("the final attempt always returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, io::Write};

    #[test]
    fn transient_replacement_errors_keep_the_old_document_until_atomic_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("document.json");
        std::fs::write(&path, b"old").unwrap();
        let mut staging = NamedTempFile::new_in(dir.path()).unwrap();
        staging.write_all(b"new").unwrap();
        let original = staging.path().to_owned();
        let calls = Cell::new(0);
        let pauses = Cell::new(0);
        retry(
            staging,
            3,
            |file| {
                assert_eq!(file.path(), original);
                assert_eq!(std::fs::read(&path).unwrap(), b"old");
                assert_eq!(std::fs::read(file.path()).unwrap(), b"new");
                let call = calls.get();
                calls.set(call + 1);
                if call < 3 {
                    Err(PersistError {
                        error: io::Error::from_raw_os_error([5, 32, 33][call]),
                        file,
                    })
                } else {
                    file.persist(&path).map(|_| ())
                }
            },
            || pauses.set(pauses.get() + 1),
        )
        .unwrap();
        assert_eq!((calls.get(), pauses.get()), (4, 3));
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!original.exists());
    }

    #[test]
    fn exhausted_or_unrelated_failures_do_not_delete_the_surviving_document() {
        for (code, retries, expected_calls) in [(5, 2, 3), (32, 0, 1), (22, 20, 1)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("document.json");
            std::fs::write(&path, b"old").unwrap();
            let staging = NamedTempFile::new_in(dir.path()).unwrap();
            let original = staging.path().to_owned();
            let calls = Cell::new(0);
            let pauses = Cell::new(0);
            let error = retry(
                staging,
                retries,
                |file| {
                    calls.set(calls.get() + 1);
                    Err(PersistError {
                        error: io::Error::from_raw_os_error(code),
                        file,
                    })
                },
                || pauses.set(pauses.get() + 1),
            )
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(code));
            assert_eq!(
                (calls.get(), pauses.get()),
                (expected_calls, expected_calls - 1)
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            assert!(!original.exists(), "failed staging file was leaked");
        }
    }
}
