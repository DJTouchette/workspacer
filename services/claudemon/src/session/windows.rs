//! The context window: one table, one resolver.
//!
//! Before this module the repo held FIVE hand-maintained window tables — this
//! daemon's `providers::context_window_for`, its `requested_context_window_for`,
//! `pricing::BUILTIN`'s `context_limit`, the desktop's `MODEL_RATES`, and the
//! spawn-picker badge strings in `claudeModels.ts` + `cmd/brain/models.go` — and
//! three of them disagreed for the same model id. A `gpt-5-codex` session read
//! 272_000 through the provider status line and 200_000 through `usage`, at the
//! same instant, for the same session, depending on which client asked.
//!
//! TWINS: `apps/desktop/src/main/services/modelContextWindows.ts` (TS) and
//! `services/hub/cmd/brain/windows.go` (Go). All three are pinned to
//! `contracts/model-context-windows.json` by the tests at the bottom of this
//! file and their opposite numbers; edit one table and the others go red.
//!
//! The second thing this module owns is the shape of NOT KNOWING. A window is
//! `Option<u64>` and `None` means "we do not know", which is a different fact
//! from any number. It used to be spelled `200_000` in four places — including
//! the empty usage a pane renders before a single token has been counted — and
//! that is the whole of the "every session starts at 200k and upgrades later"
//! complaint.

use serde::{Deserialize, Serialize};

/// Fresh-Codex spawn request from the cross-language contract fixture's
/// `providerContextDefaults` block. It is intentionally distinct from the
/// model lookup table: this is a requested CLI override, not a reported window.
pub const DEFAULT_CODEX_CONTEXT_WINDOW: u64 = 1_000_000;

/// How a table row matches a model id (already lowercased).
///
/// Normative, not an implementation detail: the `o3` / `o4` rows are `Prefix`
/// rows precisely so an `o3` buried inside an unrelated id does not match, and
/// a port that read every row as a substring would pass every lookup case in
/// the fixture except that one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Contains,
    Prefix,
    Suffix,
}

struct WindowRow {
    /// Lowercased needle.
    needle: &'static str,
    kind: MatchKind,
    window: u64,
}

const fn contains(needle: &'static str, window: u64) -> WindowRow {
    WindowRow {
        needle,
        kind: MatchKind::Contains,
        window,
    }
}

const fn prefix(needle: &'static str, window: u64) -> WindowRow {
    WindowRow {
        needle,
        kind: MatchKind::Prefix,
        window,
    }
}

const fn suffix(needle: &'static str, window: u64) -> WindowRow {
    WindowRow {
        needle,
        kind: MatchKind::Suffix,
        window,
    }
}

/// THE TABLE, in order: first match wins, so the specific rows come before the
/// families they overlap. Pinned row-for-row to the fixture's `windows` block.
const WINDOWS: &[WindowRow] = &[
    // Marker rows first — a statement about the WINDOW outranks a statement
    // about the family, whichever family carries it.
    suffix("[1m]", 1_000_000),
    suffix("-1m", 1_000_000),
    // 1M-native: the max window is also the default, so these ids never carry a
    // marker. Before the generic claude row or their gauges read 5× too full.
    contains("fable", 1_000_000),
    contains("mythos", 1_000_000),
    contains("gemini", 1_048_576),
    contains("gpt-4.1", 1_047_576),
    // Table KNOWLEDGE, not a fallback: an unmarked Claude model really does
    // hold 200k. The four places that spelled *unknown* 200_000 are gone.
    contains("claude", 200_000),
    contains("gpt-5", 272_000),
    contains("codex", 272_000),
    contains("gpt-4o", 128_000),
    prefix("o3", 200_000),
    prefix("o4", 200_000),
    contains("/o3", 200_000),
    contains("/o4", 200_000),
    contains("grok", 256_000),
    contains("deepseek", 131_072),
    contains("kimi", 262_144),
    contains("qwen", 262_144),
];

/// How far past the claimed window a session may be observed before we stop
/// believing the claim. Two percent absorbs the provider's own rounding.
const DRIFT_TOLERANCE_NUM: u64 = 102;
const DRIFT_TOLERANCE_DEN: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub model: String,
    pub context_window: Option<u64>,
}

/// The canonical selection plus the exact compatibility spelling persisted in
/// `sessions.requested_model`. Keeping the two in one value makes it impossible
/// for a normal writer to update only one side of the dual-write contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedModelSelection {
    pub selection: ModelSelection,
    pub legacy_model: String,
}

impl PersistedModelSelection {
    /// Pair an already-validated canonical selection with its compatibility
    /// spelling. Read recovery uses this when a row has canonical data but a
    /// missing legacy companion; the next real write can then heal both sides.
    pub fn from_selection(selection: ModelSelection) -> Self {
        let legacy_model = legacy_model_for_normalized_selection(&selection);
        Self {
            selection,
            legacy_model,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSelectionError {
    EmptyModel,
    InvalidModelIdentity,
    InvalidContextWindow,
    UnsupportedContextWindow,
    ConflictingContextWindow,
    ConflictingModelIdentity,
}

impl ModelSelectionError {
    pub fn code(self) -> &'static str {
        match self {
            Self::EmptyModel => "empty-model",
            Self::InvalidModelIdentity => "invalid-model-identity",
            Self::InvalidContextWindow => "invalid-context-window",
            Self::UnsupportedContextWindow => "unsupported-context-window",
            Self::ConflictingContextWindow => "conflicting-context-window",
            Self::ConflictingModelIdentity => "conflicting-model-identity",
        }
    }
}

fn strip_legacy_one_million_suffix(model: &str) -> Option<&str> {
    let lower = model.to_ascii_lowercase();
    if lower.ends_with("[1m]") {
        Some(&model[..model.len() - 4])
    } else if lower.ends_with("-1m") {
        Some(&model[..model.len() - 3])
    } else {
        None
    }
}

fn unsafe_model_identity_char(ch: char) -> bool {
    ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}')
}

/// Canonicalize legacy model-selection syntax immediately at ingress.
/// Unknown identities are intentionally preserved; only trailing markers are
/// syntax, and successful output never contains one.
pub fn normalize_model_selection(
    model: &str,
    context_window: Option<u64>,
) -> Result<ModelSelection, ModelSelectionError> {
    let mut identity = model.trim();
    if identity.is_empty() {
        return Err(ModelSelectionError::EmptyModel);
    }
    if model.chars().any(unsafe_model_identity_char) {
        return Err(ModelSelectionError::InvalidModelIdentity);
    }

    let mut legacy_window = None;
    while let Some(base) = strip_legacy_one_million_suffix(identity) {
        identity = base.trim_end();
        legacy_window = Some(1_000_000);
    }
    if identity.is_empty() {
        return Err(ModelSelectionError::EmptyModel);
    }
    if context_window == Some(0) {
        return Err(ModelSelectionError::InvalidContextWindow);
    }
    if legacy_window.is_some() && context_window.is_some() && legacy_window != context_window {
        return Err(ModelSelectionError::ConflictingContextWindow);
    }

    Ok(ModelSelection {
        model: identity.to_string(),
        context_window: context_window.or(legacy_window),
    })
}

/// Normalize a legacy request once, then derive the legacy companion older
/// daemons understand from that canonical result. Native-1M identities stay
/// bare; selectable Claude 1M variants retain the marker.
pub fn normalize_persisted_model_selection(
    model: &str,
) -> Result<PersistedModelSelection, ModelSelectionError> {
    let selection = normalize_model_selection(model, None)?;
    if selection
        .context_window
        .is_some_and(|value| value > i64::MAX as u64)
    {
        return Err(ModelSelectionError::InvalidContextWindow);
    }
    Ok(PersistedModelSelection::from_selection(selection))
}

/// Resolve the additive launch/switch wire. `legacy_model` is the executable
/// spelling older receivers understand; `model_identity` + `context_window`
/// are the canonical pair. A present pair is authoritative, but its companion
/// must agree so old and new receivers cannot launch different selections.
///
/// Claude alone assigns syntax to trailing `[1m]` / `-1m`. Other providers'
/// ids are opaque, so an id such as `vendor/model-1m` remains byte-for-byte.
pub fn normalize_model_input(
    provider: &str,
    legacy_model: Option<&str>,
    model_identity: Option<&str>,
    context_window: Option<u64>,
) -> Result<Option<PersistedModelSelection>, ModelSelectionError> {
    let normalized_provider = provider.trim().to_ascii_lowercase();
    if context_window.is_some() && !matches!(normalized_provider.as_str(), "claude" | "codex") {
        return Err(ModelSelectionError::UnsupportedContextWindow);
    }
    if legacy_model
        .into_iter()
        .chain(model_identity)
        .filter(|value| !value.trim().is_empty())
        .any(|value| value.chars().any(unsafe_model_identity_char))
    {
        return Err(ModelSelectionError::InvalidModelIdentity);
    }
    let legacy = legacy_model
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let identity = model_identity
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_canonical = model_identity.is_some() || context_window.is_some();

    if normalized_provider != "claude" {
        if context_window == Some(0) {
            return Err(ModelSelectionError::InvalidContextWindow);
        }
        if has_canonical && identity.is_none() && legacy.is_none() && normalized_provider != "codex"
        {
            return Err(ModelSelectionError::EmptyModel);
        }
        if let (Some(identity), Some(legacy)) = (identity, legacy) {
            if identity != legacy {
                return Err(ModelSelectionError::ConflictingModelIdentity);
            }
        }
        let Some(model) = identity.or(legacy) else {
            return Ok(None);
        };
        return Ok(Some(PersistedModelSelection {
            selection: ModelSelection {
                model: model.to_string(),
                context_window,
            },
            legacy_model: legacy.unwrap_or(model).to_string(),
        }));
    }

    let selection = if has_canonical {
        let identity = identity.or(legacy).ok_or(ModelSelectionError::EmptyModel)?;
        let canonical = normalize_model_selection(identity, context_window)?;
        // The canonical field must actually be canonical; accepting a marker
        // here would immediately reintroduce it into persistence/snapshots.
        let identity_was_canonical = model_identity
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        if identity_was_canonical && canonical.model != identity {
            return Err(ModelSelectionError::ConflictingModelIdentity);
        }
        if identity_was_canonical {
            if let Some(legacy) = legacy {
                let companion = normalize_model_selection(legacy, None)?;
                let expected_legacy = legacy_model_for_normalized_selection(&canonical);
                let expected_companion = normalize_model_selection(&expected_legacy, None)?;
                if companion != expected_companion {
                    return Err(ModelSelectionError::ConflictingModelIdentity);
                }
            }
        }
        canonical
    } else if let Some(legacy) = legacy {
        normalize_model_selection(legacy, None)?
    } else {
        return Ok(None);
    };
    Ok(Some(PersistedModelSelection::from_selection(selection)))
}

/// Read adapter for SQLite rows. Canonical data is authoritative when its
/// backward-compatible legacy projection agrees with the legacy column. This
/// matters for native-1M Fable/Mythos: their canonical identity is bare, while
/// a canonical 1M window deliberately projects to a bare legacy value. When
/// the projections genuinely differ, the legacy value is newer rollback-era
/// evidence: a v8 daemon can update only `requested_model`, so preferring the
/// stale canonical pair would silently undo that model switch. Missing/invalid
/// evidence on either side falls back to the other without making the
/// surrounding row unreadable.
///
/// `context_window` is signed because SQLite can contain manually-corrupted or
/// future values. Non-positive values are invalid canonical evidence and must
/// not make the whole session row unreadable.
pub fn restore_persisted_model_selection(
    identity: Option<&str>,
    context_window: Option<i64>,
    legacy_model: Option<&str>,
) -> Option<ModelSelection> {
    let canonical = identity.and_then(|identity| {
        let identity = identity.trim();
        if identity.is_empty() || canonical_identity_has_legacy_marker(identity) {
            return None;
        }
        let context_window = match context_window {
            Some(value) => Some(u64::try_from(value).ok().filter(|value| *value > 0)?),
            None => None,
        };
        Some(ModelSelection {
            // Canonical provider identities are opaque. A non-Claude model may
            // legitimately end in `-1m`; only provider-aware ingress is allowed
            // to interpret Claude compatibility syntax.
            model: identity.to_string(),
            context_window,
        })
    });

    let legacy = legacy_model.and_then(|model| normalize_model_selection(model, None).ok());
    match (canonical, legacy) {
        // The sessions table predates provider persistence. Exact agreement is
        // provider-neutral proof for opaque identities, including a genuine
        // non-Claude `vendor/model-1m`. It is NOT sufficient for a known Claude
        // selectable 1M identity: `opus` + 1M projects to `opus[1m]`, so a bare
        // legacy `opus` is newer v8 rollback evidence selecting the base model.
        (Some(canonical), Some(_))
            if legacy_model.is_some_and(|legacy| legacy.trim() == canonical.model)
                && (legacy_model_for_normalized_selection(&canonical) == canonical.model
                    || !is_recognizable_claude_identity(&canonical.model)) =>
        {
            Some(canonical)
        }
        (Some(canonical), Some(legacy))
            if legacy_model_for_normalized_selection(&canonical)
                == legacy_model_for_normalized_selection(&legacy) =>
        {
            Some(canonical)
        }
        // A prior v8 daemon writes only requested_model. A disagreement therefore
        // means that compatibility value was written after the canonical pair.
        (_, Some(legacy)) => Some(legacy),
        (Some(canonical), None) => Some(canonical),
        (None, None) => None,
    }
}

fn is_recognizable_claude_identity(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.starts_with("claude-")
        || matches!(
            lower.as_str(),
            "opus" | "sonnet" | "haiku" | "fable" | "mythos"
        )
}

fn canonical_identity_has_legacy_marker(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    if lower.ends_with("[1m]") {
        return true;
    }
    lower
        .strip_suffix("-1m")
        .is_some_and(is_recognizable_claude_identity)
}

fn legacy_model_for_normalized_selection(selection: &ModelSelection) -> String {
    if selection.context_window == Some(1_000_000)
        && !is_claude_inherent_one_million_model(&selection.model)
    {
        format!("{}[1m]", selection.model)
    } else {
        selection.model.clone()
    }
}

/// Claude Code's external `--model` value. This is the one boundary allowed to
/// reconstruct `[1m]`; normalizing first keeps legacy callers idempotent.
pub fn claude_argv_model(selection: &ModelSelection) -> Result<String, ModelSelectionError> {
    let normalized = normalize_model_selection(&selection.model, selection.context_window)?;
    Ok(legacy_model_for_normalized_selection(&normalized))
}

/// Fable/Mythos expose 1M as their inherent window, not as a selectable
/// marker-bearing variant. The shared argv fixture pins this with the TS and
/// Go boundaries so a port cannot invent `fable[1m]` or `mythos[1m]`.
fn is_claude_inherent_one_million_model(model: &str) -> bool {
    let identity = model.to_ascii_lowercase();
    identity.contains("fable") || identity.contains("mythos")
}

/// The table's answer for a concrete model id, or `None` when no row covers it.
///
/// `None` is the honest unknown and it is load-bearing: every readout in the
/// repo already hides its meter on an absent window, so a model this table has
/// never heard of shows no meter instead of a familiar-looking wrong one.
pub fn window_for(model: &str) -> Option<u64> {
    let m = model.to_ascii_lowercase();
    for row in WINDOWS {
        let hit = match row.kind {
            MatchKind::Contains => m.contains(row.needle),
            MatchKind::Prefix => m.starts_with(row.needle),
            MatchKind::Suffix => m.ends_with(row.needle),
        };
        if hit {
            return Some(row.window);
        }
    }
    None
}

/// The window implied by the model string a session was *asked* for — the alias
/// the user picked (`opus[1m]`), not the concrete id the transcript records.
///
/// Deliberately narrower than [`window_for`]: it answers only "was 1M asked
/// for", and `None` means "says nothing", NOT "200k". A bare `opus` may be a
/// 200k session or whatever Claude Code's default becomes tomorrow; pinning a
/// number here is exactly how a wrong window gets asserted from token zero.
pub fn requested_window_for(model: &str) -> Option<u64> {
    let selection = normalize_model_selection(model, None).ok()?;
    requested_window_for_selection(&selection)
}

/// The requested-window claim from an already-normalized canonical selection.
pub fn requested_window_for_selection(selection: &ModelSelection) -> Option<u64> {
    let m = selection.model.to_ascii_lowercase();
    if selection.context_window == Some(1_000_000) || m.contains("fable") || m.contains("mythos") {
        return Some(1_000_000);
    }
    None
}

/// THE RESOLVER. One hierarchy, and the order is the design:
///
///   1. `reported` — the window the provider itself gave for THIS session
///      (Claude's statusLine `context_window_size`, the stream `result` frame's
///      `modelUsage.*.contextWindow`, Codex's `model_context_window`). A fact.
///   2. `override_window` — the user's `~/.workspacer/model-rates.json`
///      `context_limit`. They are overruling us deliberately, so it outranks
///      the marker below. This REPLACED a `max()`, under which a coarse alias
///      could silently raise the window back over what the user wrote.
///   3. `requested` — the `[1m]` marker on the model asked for at spawn. Known
///      from token zero, which is what makes birth-time knowledge possible;
///      Claude Code strips the marker from the id it records, so the request is
///      the only carrier.
///   4. the contract table, by concrete model id.
///   5. `None` — unknown. Never 200_000.
///
/// Then the DRIFT ALARM. `peak_context` is the session's high-water context
/// occupancy; a claim it EXCEEDS (past a 2% tolerance) is demonstrably wrong,
/// so that claim is DISQUALIFIED and the next one down the hierarchy is tried.
/// Unknown is the answer only once every claim has been disproved. This is the
/// retrospective 200k→1M promotion, demoted from a source of truth to an alarm:
/// it used to silently REWRITE the window to 1M, which is a guess dressed as a
/// correction.
///
/// Disqualify-and-CONTINUE, not disqualify-and-stop. Stopping threw away claims
/// the evidence had never touched, and that cost a real reading: Claude Code's
/// own statusLine reports `context_window_size: 200000` for a session spawned
/// `opus[1m]`, so a live 1M worker holding 356k tokens had its REPORTED window
/// disproved and then resolved to UNKNOWN — instead of falling through to the
/// `[1m]` marker, which says 1M and which 356k does not contradict. An unknown
/// window is what made that worker's context bar peg at 100%. Falling through
/// invents nothing: every candidate below already existed and was already
/// ranked; the alarm now only removes the ones the session disproved.
pub fn resolve_window(
    model: Option<&str>,
    requested: Option<&str>,
    reported: Option<u64>,
    override_window: Option<u64>,
    peak_context: u64,
) -> Option<u64> {
    let requested_spelling = requested;
    let requested = requested.and_then(|value| normalize_model_selection(value, None).ok());
    resolve_window_for_selection_with_requested_spelling(
        model,
        requested.as_ref(),
        requested_spelling,
        reported,
        override_window,
        peak_context,
    )
}

/// [`resolve_window`] for callers that already own the canonical selection.
/// This is the daemon persistence/session boundary's read path; it deliberately
/// does not round-trip through `requested_model`.
pub fn resolve_window_for_selection(
    model: Option<&str>,
    requested: Option<&ModelSelection>,
    reported: Option<u64>,
    override_window: Option<u64>,
    peak_context: u64,
) -> Option<u64> {
    resolve_window_for_selection_with_requested_spelling(
        model,
        requested,
        requested.map(|selection| selection.model.as_str()),
        reported,
        override_window,
        peak_context,
    )
}

/// [`resolve_window_for_selection`] with the original requested-model spelling
/// retained for diagnostics. The canonical selection is the source of window
/// truth; the raw spelling is only surfaced in the drift warning so an operator
/// can see the model value the caller actually supplied.
pub(crate) fn resolve_window_for_selection_with_requested_spelling(
    model: Option<&str>,
    requested: Option<&ModelSelection>,
    requested_spelling: Option<&str>,
    reported: Option<u64>,
    override_window: Option<u64>,
    peak_context: u64,
) -> Option<u64> {
    let claims = window_claims(model, requested, reported, override_window);
    for claim in claims {
        if peak_context > claim.saturating_mul(DRIFT_TOLERANCE_NUM) / DRIFT_TOLERANCE_DEN {
            tracing::warn!(
                model = model.unwrap_or("<none>"),
                requested = requested_spelling.unwrap_or("<none>"),
                claimed_window = claim,
                observed_peak = peak_context,
                "context window drift: this session holds more than a window claimed for it, \
                 so that claim is disqualified and the next one is tried. If every claim is \
                 disproved the window is reported as UNKNOWN, and if this model is real, \
                 contracts/model-context-windows.json is stale for it."
            );
            continue;
        }
        return Some(claim);
    }
    None
}

/// The hierarchy, in order, as a LIST rather than a single answer — so the
/// alarm can drop a disproved claim and keep going. Split out so the alarm has
/// something to compare against and so tests can name the two halves separately.
fn window_claims(
    model: Option<&str>,
    requested: Option<&ModelSelection>,
    reported: Option<u64>,
    override_window: Option<u64>,
) -> Vec<u64> {
    let mut out = Vec::with_capacity(4);
    out.extend(reported.filter(|w| *w > 0));
    out.extend(override_window.filter(|w| *w > 0));
    out.extend(requested.and_then(requested_window_for_selection));
    out.extend(model.and_then(window_for));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    // The fixture is compiled in so a `cargo test` that never touches the repo
    // root still fails when the table drifts from its twins.
    const FIXTURE: &str = include_str!("../../../../contracts/model-context-windows.json");

    #[derive(Deserialize)]
    struct Fixture {
        #[serde(rename = "providerContextDefaults")]
        provider_context_defaults: Vec<ProviderContextDefault>,
        windows: Vec<WindowCase>,
        #[serde(rename = "lookupCases")]
        lookup_cases: Vec<LookupCase>,
        #[serde(rename = "markerCases")]
        marker_cases: Vec<MarkerCase>,
        #[serde(rename = "selectionCases")]
        selection_cases: Vec<SelectionCase>,
        #[serde(rename = "claudeArgvCases")]
        claude_argv_cases: Vec<ClaudeArgvCase>,
        #[serde(rename = "inputCases")]
        input_cases: Vec<InputCase>,
        #[serde(rename = "resolutionCases")]
        resolution_cases: Vec<ResolutionCase>,
    }

    #[derive(Deserialize)]
    struct ProviderContextDefault {
        provider: String,
        #[serde(rename = "freshContextWindow")]
        fresh_context_window: u64,
        note: String,
    }

    #[derive(Deserialize)]
    struct WindowCase {
        #[serde(rename = "match")]
        needle: String,
        kind: String,
        window: u64,
    }

    #[derive(Deserialize)]
    struct LookupCase {
        model: String,
        expected: Option<u64>,
        note: String,
    }

    #[derive(Deserialize)]
    struct MarkerCase {
        requested: String,
        expected: Option<u64>,
        note: String,
    }

    #[derive(Deserialize)]
    struct ResolutionCase {
        name: String,
        model: Option<String>,
        #[serde(rename = "requestedModel")]
        requested_model: Option<String>,
        #[serde(rename = "reportedWindow")]
        reported_window: Option<u64>,
        #[serde(rename = "override")]
        override_window: Option<u64>,
        #[serde(rename = "peakContext")]
        peak_context: u64,
        expected: Option<u64>,
        note: String,
    }

    #[derive(Deserialize)]
    struct SelectionCase {
        name: String,
        model: String,
        #[serde(rename = "contextWindow")]
        context_window: Option<u64>,
        #[serde(rename = "expectedModel")]
        expected_model: Option<String>,
        #[serde(rename = "expectedContextWindow")]
        expected_context_window: Option<u64>,
        error: Option<String>,
        note: String,
    }

    #[derive(Deserialize)]
    struct ClaudeArgvCase {
        name: String,
        model: String,
        #[serde(rename = "contextWindow")]
        context_window: Option<u64>,
        expected: Option<String>,
        error: Option<String>,
        note: String,
    }

    #[derive(Deserialize)]
    struct InputCase {
        name: String,
        provider: String,
        model: Option<String>,
        #[serde(rename = "modelIdentity")]
        model_identity: Option<String>,
        #[serde(rename = "contextWindow")]
        context_window: Option<u64>,
        #[serde(rename = "expectedModel")]
        expected_model: Option<String>,
        #[serde(rename = "expectedContextWindow")]
        expected_context_window: Option<u64>,
        #[serde(rename = "expectedLegacyModel")]
        expected_legacy_model: Option<String>,
        error: Option<String>,
        note: String,
    }

    fn fixture() -> Fixture {
        serde_json::from_str(FIXTURE).expect("model-context-windows.json parses")
    }

    #[test]
    fn provider_context_defaults_match_the_contract() {
        let f = fixture();
        assert_eq!(
            f.provider_context_defaults.len(),
            1,
            "provider defaults were removed"
        );
        let codex = f
            .provider_context_defaults
            .iter()
            .find(|row| row.provider == "codex")
            .expect("the contract must name Codex");
        assert_eq!(
            DEFAULT_CODEX_CONTEXT_WINDOW, codex.fresh_context_window,
            "Codex default drifted from the contract — {}",
            codex.note
        );
    }

    /// The table itself, row for row and IN ORDER. A lookup corpus alone would
    /// not catch a reordering that happens to leave every sampled id where it
    /// was; the order is what makes `fable` beat `claude`.
    #[test]
    fn the_table_matches_the_contract_row_for_row() {
        let f = fixture();
        assert_eq!(
            f.windows.len(),
            WINDOWS.len(),
            "the contract has {} rows and this table has {} — a row was added on one side only",
            f.windows.len(),
            WINDOWS.len()
        );
        for (i, (want, got)) in f.windows.iter().zip(WINDOWS.iter()).enumerate() {
            assert_eq!(want.needle, got.needle, "row {i}: needle");
            let want_kind = match want.kind.as_str() {
                "contains" => MatchKind::Contains,
                "prefix" => MatchKind::Prefix,
                "suffix" => MatchKind::Suffix,
                other => panic!("row {i}: unknown match kind {other:?} in the fixture"),
            };
            assert_eq!(want_kind, got.kind, "row {i} ({}): match kind", want.needle);
            assert_eq!(want.window, got.window, "row {i} ({}): window", want.needle);
        }
    }

    #[test]
    fn lookup_cases_resolve_as_the_contract_says() {
        let f = fixture();
        assert!(f.lookup_cases.len() >= 20, "the corpus was gutted");
        for c in &f.lookup_cases {
            assert_eq!(
                window_for(&c.model),
                c.expected,
                "window_for({:?}) — {}",
                c.model,
                c.note
            );
        }
    }

    #[test]
    fn marker_cases_resolve_as_the_contract_says() {
        let f = fixture();
        assert!(f.marker_cases.len() >= 8, "the corpus was gutted");
        for c in &f.marker_cases {
            assert_eq!(
                requested_window_for(&c.requested),
                c.expected,
                "requested_window_for({:?}) — {}",
                c.requested,
                c.note
            );
        }
    }

    #[test]
    fn selection_cases_follow_the_contract() {
        let f = fixture();
        assert!(f.selection_cases.len() >= 10, "the corpus was gutted");
        for c in &f.selection_cases {
            match normalize_model_selection(&c.model, c.context_window) {
                Ok(got) => {
                    assert!(
                        c.error.is_none(),
                        "{} unexpectedly succeeded — {}",
                        c.name,
                        c.note
                    );
                    assert_eq!(
                        Some(got.model.clone()),
                        c.expected_model,
                        "{} — {}",
                        c.name,
                        c.note
                    );
                    assert_eq!(
                        got.context_window, c.expected_context_window,
                        "{} — {}",
                        c.name, c.note
                    );
                    let lower = got.model.to_ascii_lowercase();
                    assert!(!lower.ends_with("[1m]") && !lower.ends_with("-1m"));
                    assert_eq!(
                        normalize_model_selection(&got.model, got.context_window).unwrap(),
                        got,
                        "{} is not idempotent",
                        c.name
                    );
                }
                Err(err) => assert_eq!(
                    Some(err.code()),
                    c.error.as_deref(),
                    "{} — {}",
                    c.name,
                    c.note
                ),
            }
        }
    }

    #[test]
    fn input_cases_follow_the_contract() {
        let f = fixture();
        assert!(f.input_cases.len() >= 12, "the input corpus was gutted");
        for c in &f.input_cases {
            match normalize_model_input(
                &c.provider,
                c.model.as_deref(),
                c.model_identity.as_deref(),
                c.context_window,
            ) {
                Ok(got) => {
                    assert!(
                        c.error.is_none(),
                        "{} unexpectedly succeeded — {}",
                        c.name,
                        c.note
                    );
                    match (&c.expected_model, got) {
                        (None, None) => {}
                        (Some(expected_model), Some(got)) => {
                            assert_eq!(
                                &got.selection.model, expected_model,
                                "{} — {}",
                                c.name, c.note
                            );
                            assert_eq!(
                                got.selection.context_window, c.expected_context_window,
                                "{} — {}",
                                c.name, c.note
                            );
                            assert_eq!(
                                Some(got.legacy_model),
                                c.expected_legacy_model,
                                "{} — {}",
                                c.name,
                                c.note
                            );
                        }
                        (_, got) => panic!(
                            "{}: got {got:?}, expected model {:?} — {}",
                            c.name, c.expected_model, c.note
                        ),
                    }
                }
                Err(err) => assert_eq!(
                    Some(err.code().to_string()),
                    c.error,
                    "{} — {}",
                    c.name,
                    c.note
                ),
            }
        }
    }

    #[test]
    fn claude_argv_cases_follow_the_contract() {
        let f = fixture();
        assert!(f.claude_argv_cases.len() >= 4, "the corpus was gutted");
        for c in &f.claude_argv_cases {
            let got = claude_argv_model(&ModelSelection {
                model: c.model.clone(),
                context_window: c.context_window,
            });
            match got {
                Ok(value) => {
                    assert!(
                        c.error.is_none(),
                        "{} unexpectedly succeeded — {}",
                        c.name,
                        c.note
                    );
                    assert_eq!(Some(value), c.expected, "{} — {}", c.name, c.note);
                }
                Err(err) => assert_eq!(
                    Some(err.code()),
                    c.error.as_deref(),
                    "{} — {}",
                    c.name,
                    c.note
                ),
            }
        }
    }

    /// The block that actually pins the twins: it exercises the RESOLVER, so a
    /// stack that ports the table correctly but the hierarchy wrong still goes
    /// red.
    #[test]
    fn resolution_cases_follow_the_hierarchy() {
        let f = fixture();
        assert!(f.resolution_cases.len() >= 12, "the corpus was gutted");
        for c in &f.resolution_cases {
            let got = resolve_window(
                c.model.as_deref(),
                c.requested_model.as_deref(),
                c.reported_window,
                c.override_window,
                c.peak_context,
            );
            assert_eq!(got, c.expected, "{} — {}", c.name, c.note);
        }
    }

    /// The alarm must not be reachable only through the fixture: assert the
    /// boundary directly, because "off by one on the tolerance" is the way a
    /// full 200k session loses its meter at exactly the moment it matters.
    #[test]
    fn the_drift_alarm_fires_only_past_the_tolerance() {
        let at = |peak| resolve_window(Some("claude-opus-5"), None, None, None, peak);
        assert_eq!(at(200_000), Some(200_000), "exactly full is not over-full");
        assert_eq!(at(204_000), Some(200_000), "1.02× exactly is the boundary");
        assert_eq!(at(204_001), None, "past it, the window is not believable");
    }
}
