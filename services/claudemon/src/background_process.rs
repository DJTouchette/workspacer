//! Console suppression for pipe-driven helpers and providers. ConPTY and
//! explicitly visible terminals retain their own process creation policy.
#[cfg(windows)]
pub const CREATION_FLAGS: u32 = windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

pub trait BackgroundCommand {
    fn no_console_window(&mut self) -> &mut Self;
}

impl BackgroundCommand for std::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(CREATION_FLAGS);
        }
        self
    }
}

impl BackgroundCommand for tokio::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        self.as_std_mut().no_console_window();
        self
    }
}
