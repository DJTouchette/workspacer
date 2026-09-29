use std::path::Path;
/// CLI retains the Go launcher's config-first precedence. None deliberately
/// leaves the engine's ambient environment/default intact. Native uses its own
/// existing environment-first helper and is not changed by this boundary.
pub(super) fn configured(directory: &Path) -> Option<bool> {
    setting(&std::fs::read_to_string(directory.join("config.yaml")).ok()?)
}
fn setting(raw: &str) -> Option<bool> {
    let value: serde_yaml::Value = serde_yaml::from_str(raw).ok()?;
    value.get("usage")?.get("pollOnBoot")?.as_bool()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_cli_setting_preserves_boolean_presence_instead_of_inventing_a_default() {
        for (raw, expected) in [
            ("usage:\n  pollOnBoot: false\n", Some(false)),
            ("usage:\n  pollOnBoot: true\n", Some(true)),
            ("", None),
            ("usage: {}", None),
            ("usage: {pollOnBoot: 'false'}", None),
            ("usage:\n\tpollOnBoot: false", None),
            ("usage: false", None),
        ] {
            assert_eq!(setting(raw), expected, "{raw}");
        }
        let root = tempfile::tempdir().unwrap();
        assert_eq!(configured(root.path()), None);
        std::fs::write(
            root.path().join("config.yaml"),
            "usage: {pollOnBoot: false}",
        )
        .unwrap();
        assert_eq!(configured(root.path()), Some(false));
        std::fs::write(root.path().join("config.yaml"), "usage: {pollOnBoot: true}").unwrap();
        assert_eq!(configured(root.path()), Some(true));
    }
}
