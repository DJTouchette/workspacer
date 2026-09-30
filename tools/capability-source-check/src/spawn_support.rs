//! Explicit support dispositions are separate from source reads and key spelling.
use crate::{Index, Report};
use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Side {
    pub observed: bool,
    pub disposition: String,
    pub reason: String,
    pub evidence_kind: String,
    pub evidence: Vec<String>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub name: String,
    pub rust: Side,
    pub desktop: Side,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Difference {
    pub rust: String,
    pub desktop: String,
    pub reason: String,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Differences {
    pub rust_only: BTreeMap<String, String>,
    pub desktop_only: BTreeMap<String, String>,
    pub semantics: BTreeMap<String, Difference>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub cases: Vec<Row>,
    pub mirrored_supported: Vec<String>,
    pub differences: Differences,
}
fn set(values: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    values.into_iter().collect()
}
pub fn structure(policy: &Policy) -> Vec<String> {
    let mut errors = vec![];
    let names = set(policy.cases.iter().map(|r| r.name.clone()));
    if names.len() != policy.cases.len() || names.len() < 40 {
        errors.push("support policy population collapsed or duplicated".into());
    }
    for row in &policy.cases {
        if !row.rust.observed && !row.desktop.observed {
            errors.push(format!("stale unobserved support row {}", row.name));
        }
        for (label, side) in [("rust", &row.rust), ("desktop", &row.desktop)] {
            let kind = match side.disposition.as_str() {
                "supported" => "implementation",
                "refused" => "refusal",
                "host-derived" => "host-value",
                "ignored" => "compatibility-policy",
                _ => "invalid",
            };
            if kind == "invalid"
                || side.evidence_kind != kind
                || side.reason.trim().is_empty()
                || side.evidence.is_empty()
            {
                errors.push(format!("unreviewed {label} disposition {}", row.name));
            }
        }
    }
    let mirrored = set(policy
        .cases
        .iter()
        .filter(|r| r.rust.disposition == "supported" && r.desktop.disposition == "supported")
        .map(|r| r.name.clone()));
    if mirrored != set(policy.mirrored_supported.clone())
        || policy.mirrored_supported.len() != mirrored.len()
    {
        errors.push("mirrored supported intersection drifted".into());
    }
    for (label, actual, declared) in [
        (
            "rust-only",
            set(policy
                .cases
                .iter()
                .filter(|r| r.rust.observed && !r.desktop.observed)
                .map(|r| r.name.clone())),
            &policy.differences.rust_only,
        ),
        (
            "desktop-only",
            set(policy
                .cases
                .iter()
                .filter(|r| r.desktop.observed && !r.rust.observed)
                .map(|r| r.name.clone())),
            &policy.differences.desktop_only,
        ),
    ] {
        if actual != set(declared.keys().cloned()) || declared.values().any(|r| r.trim().is_empty())
        {
            errors.push(format!("stale or missing {label} support explanation"));
        }
    }
    let mismatched = set(policy
        .cases
        .iter()
        .filter(|r| r.rust.disposition != r.desktop.disposition)
        .map(|r| r.name.clone()));
    if mismatched != set(policy.differences.semantics.keys().cloned()) {
        errors.push("stale or missing semantic support difference".into());
    }
    for row in &policy.cases {
        if let Some(reason) = policy.differences.semantics.get(&row.name) {
            if reason.rust != row.rust.disposition
                || reason.desktop != row.desktop.disposition
                || reason.reason.trim().is_empty()
            {
                errors.push(format!("contradictory support difference {}", row.name));
            }
        }
    }
    errors
}
fn tokens(stream: TokenStream) -> Vec<String> {
    let mut out = vec![];
    for token in stream {
        match token {
            TokenTree::Group(group) => {
                out.push(format!("open:{:?}", group.delimiter()));
                out.extend(tokens(group.stream()));
                out.push(format!("close:{:?}", group.delimiter()));
            }
            TokenTree::Ident(i) => out.push(format!("id:{i}")),
            TokenTree::Punct(p) => out.push(format!("punct:{}", p.as_char())),
            TokenTree::Literal(l) => out.push(format!("literal:{l}")),
        }
    }
    out
}
pub fn check(report: &Report, index: &Index, policy: &Policy) -> Vec<String> {
    let mut errors = structure(policy);
    let mut actual = set(report
        .methods
        .get("agents.spawn")
        .into_iter()
        .flat_map(|b| b.fields.iter())
        .map(|f| f.split('.').next().unwrap().to_owned()));
    match workflow_roots(index) {
        Ok(fields) => actual.extend(fields),
        Err(error) => errors.push(error),
    }
    let expected = set(policy
        .cases
        .iter()
        .filter(|r| r.rust.observed)
        .map(|r| r.name.clone()));
    if actual.len() < 40 || actual != expected {
        errors.push(format!(
            "Rust support source closure drift: missing review {:?}, stale review {:?}",
            actual.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&actual).collect::<Vec<_>>()
        ));
    }
    for row in &policy.cases {
        for evidence in &row.rust.evidence {
            let Some((function, needle)) = evidence.split_once('|') else {
                errors.push(format!("invalid Rust support evidence {}", row.name));
                continue;
            };
            let Some(function) = index.functions.get(function) else {
                errors.push(format!(
                    "missing production support function {}: {function}",
                    row.name
                ));
                continue;
            };
            let Ok(needle) = needle.parse::<TokenStream>() else {
                errors.push(format!("invalid support code anchor {}", row.name));
                continue;
            };
            let needle = tokens(needle);
            let body = tokens(function.block.to_token_stream());
            if needle.is_empty() || !body.windows(needle.len()).any(|w| w == needle) {
                errors.push(format!(
                    "Rust support evidence changed {}: {evidence}",
                    row.name
                ));
            }
        }
    }
    errors
}

/// Inspect actual workflow caller reads, including literal iterator keys.
pub fn workflow_roots(index: &Index) -> Result<BTreeSet<String>, String> {
    use syn::visit::{self, Visit};
    struct Reads {
        fields: BTreeSet<String>,
        dynamic: bool,
        keys: BTreeMap<String, Vec<String>>,
    }
    fn parameter(e: &syn::Expr) -> bool {
        matches!(e, syn::Expr::Path(p) if p.path.is_ident("params"))
    }
    impl Reads {
        fn key(&mut self, e: &syn::Expr) {
            if let syn::Expr::Lit(l) = e {
                if let syn::Lit::Str(s) = &l.lit {
                    self.fields.insert(s.value());
                    return;
                }
            }
            if let syn::Expr::Path(p) = e {
                if let Some(i) = p.path.get_ident() {
                    if let Some(keys) = self.keys.get(&i.to_string()) {
                        self.fields.extend(keys.clone());
                        return;
                    }
                }
            }
            self.dynamic = true;
        }
    }
    impl<'a> Visit<'a> for Reads {
        fn visit_expr_index(&mut self, e: &'a syn::ExprIndex) {
            if parameter(&e.expr) {
                self.key(&e.index);
            }
            visit::visit_expr_index(self, e);
        }
        fn visit_expr_method_call(&mut self, e: &'a syn::ExprMethodCall) {
            if parameter(&e.receiver)
                && matches!(
                    e.method.to_string().as_str(),
                    "get" | "get_mut" | "remove" | "contains_key"
                )
            {
                if let Some(key) = e.args.first() {
                    self.key(key);
                }
            }
            if let syn::Expr::MethodCall(iter) = &*e.receiver {
                if iter.method == "iter" {
                    if let syn::Expr::Array(array) = &*iter.receiver {
                        if let Some(syn::Expr::Closure(c)) = e.args.first() {
                            if let Some(syn::Pat::Ident(name)) = c.inputs.first() {
                                let values: Option<Vec<String>> = array
                                    .elems
                                    .iter()
                                    .map(|v| {
                                        if let syn::Expr::Lit(l) = v {
                                            if let syn::Lit::Str(s) = &l.lit {
                                                Some(s.value())
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if let Some(values) = values {
                                    let old = self.keys.insert(name.ident.to_string(), values);
                                    self.visit_expr(&c.body);
                                    if let Some(old) = old {
                                        self.keys.insert(name.ident.to_string(), old);
                                    } else {
                                        self.keys.remove(&name.ident.to_string());
                                    }
                                    return;
                                }
                            }
                        }
                    }
                }
            }
            visit::visit_expr_method_call(self, e);
        }
        fn visit_expr_call(&mut self, e: &'a syn::ExprCall) {
            if e.args.first().is_some_and(parameter) {
                if let syn::Expr::Path(p) = &*e.func {
                    if p.path.is_ident("text") {
                        if let Some(key) = e.args.iter().nth(1) {
                            self.key(key);
                        }
                    }
                }
            }
            visit::visit_expr_call(self, e);
        }
    }
    let function = index
        .functions
        .get("services::workflow_runtime::WorkflowRuntime::admit")
        .ok_or("workflow admission function missing")?;
    let mut reads = Reads {
        fields: BTreeSet::new(),
        dynamic: false,
        keys: BTreeMap::new(),
    };
    reads.visit_block(&function.block);
    if reads.dynamic {
        return Err("unresolved workflow parameter key".into());
    }
    Ok(reads.fields)
}
