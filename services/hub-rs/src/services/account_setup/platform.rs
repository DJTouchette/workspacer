use std::path::Path;
#[cfg(unix)]
pub(crate) fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
pub(crate) fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    junction::create(target, link)
}
#[cfg(unix)]
pub(crate) fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
pub(crate) fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}
