use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use syn::{Attribute, Item, Type, UseTree};
#[derive(Clone)]
pub(crate) struct Function {
    pub key: String,
    pub module: String,
    pub owner: Option<String>,
    pub sig: syn::Signature,
    pub block: syn::Block,
}
#[derive(Clone)]
pub(crate) enum Data {
    Struct(syn::ItemStruct),
    Enum(syn::ItemEnum),
    Alias(Type),
}
pub struct Index {
    pub(crate) functions: BTreeMap<String, Function>,
    pub(crate) data: BTreeMap<String, Data>,
    pub(crate) imports: BTreeMap<String, BTreeMap<String, String>>,
    pub(crate) files: usize,
    pub(crate) constants: BTreeMap<String, syn::Expr>,
    globs: BTreeMap<String, Vec<String>>,
    pub(crate) locals: BTreeMap<String, BTreeMap<String, Data>>,
    pub(crate) ambiguous_locals: BTreeMap<String, BTreeSet<String>>,
}
pub fn read_sources(root: &Path) -> Result<BTreeMap<String, String>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
        for item in std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))? {
            let item = item.map_err(|e| e.to_string())?;
            let path = item.path();
            let kind = item.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err(format!("unaccounted source symlink {}", path.display()));
            }
            if kind.is_dir() {
                walk(root, &path, out)?
            } else if path.extension().is_some_and(|e| e == "rs") {
                let key = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(
                    key,
                    std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
                );
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, &root.join("services/hub-rs/src"), &mut out)?;
    if out.is_empty() {
        return Err("no Rust source files".into());
    }
    Ok(out)
}
fn cfg_value(meta: &syn::Meta) -> Option<bool> {
    match meta {
        syn::Meta::Path(p) if p.is_ident("test") => Some(false),
        syn::Meta::List(list) => {
            use syn::parse::Parser;
            let values = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .ok()?;
            let values: Vec<_> = values.iter().map(cfg_value).collect();
            if list.path.is_ident("all") {
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|v| *v == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else if list.path.is_ident("any") {
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|v| *v == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else if list.path.is_ident("not") && values.len() == 1 {
                values[0].map(|v| !v)
            } else {
                None
            }
        }
        _ => None,
    }
}
pub(crate) fn excluded(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("test")
            || (a.path().is_ident("cfg")
                && a.parse_args::<syn::Meta>()
                    .ok()
                    .as_ref()
                    .and_then(cfg_value)
                    == Some(false))
    })
}
fn normalize(path: &Path) -> String {
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            std::path::Component::ParentDir => {
                parts.pop();
            }
            _ => {}
        }
    }
    parts.join("/")
}
fn module_directory(file: &str) -> std::path::PathBuf {
    let path = Path::new(file);
    if matches!(
        path.file_name().and_then(|s| s.to_str()),
        Some("mod.rs" | "lib.rs" | "main.rs")
    ) {
        path.parent().unwrap().to_owned()
    } else {
        path.with_extension("")
    }
}
fn edges(
    items: &[Item],
    normal: &Path,
    attributes: &Path,
    inherited: bool,
    out: &mut Vec<(String, bool)>,
) {
    for item in items {
        let Item::Mod(m) = item else { continue };
        let test = inherited || excluded(&m.attrs);
        let explicit = m
            .attrs
            .iter()
            .find(|a| a.path().is_ident("path"))
            .and_then(|a| match &a.meta {
                syn::Meta::NameValue(v) => match &v.value {
                    syn::Expr::Lit(v) => match &v.lit {
                        syn::Lit::Str(s) => Some(attributes.join(s.value())),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            });
        if let Some((_, items)) = &m.content {
            let dir = explicit.unwrap_or_else(|| normal.join(m.ident.to_string()));
            edges(items, &dir, &dir, test, out)
        } else if let Some(path) = explicit {
            out.push((normalize(&path), test))
        } else {
            out.push((normalize(&normal.join(format!("{}.rs", m.ident))), test));
            out.push((
                normalize(&normal.join(m.ident.to_string()).join("mod.rs")),
                test,
            ));
        }
    }
}
fn imports(
    tree: &UseTree,
    prefix: &str,
    out: &mut BTreeMap<String, String>,
    globs: &mut Vec<String>,
) {
    match tree {
        UseTree::Path(p) => imports(&p.tree, &format!("{prefix}{}::", p.ident), out, globs),
        UseTree::Name(n) => {
            out.insert(n.ident.to_string(), format!("{prefix}{}", n.ident));
        }
        UseTree::Rename(n) => {
            out.insert(n.rename.to_string(), format!("{prefix}{}", n.ident));
        }
        UseTree::Group(g) => {
            for t in &g.items {
                imports(t, prefix, out, globs)
            }
        }
        UseTree::Glob(_) => globs.push(prefix.trim_end_matches("::").into()),
    }
}
impl Index {
    pub fn parse(sources: BTreeMap<String, String>) -> Result<Self, String> {
        if sources.is_empty() {
            return Err("source population empty".into());
        }
        let parsed: BTreeMap<_, _> = sources
            .iter()
            .map(|(name, text)| {
                syn::parse_file(text)
                    .map(|f| (name.clone(), f))
                    .map_err(|e| format!("{name}: {e}"))
            })
            .collect::<Result<_, _>>()?;
        let mut graph = BTreeMap::new();
        for (name, file) in &parsed {
            let mut e = Vec::new();
            edges(
                &file.items,
                &module_directory(name),
                Path::new(name).parent().unwrap(),
                false,
                &mut e,
            );
            graph.insert(name.clone(), e);
        }
        let mut tests = BTreeSet::new();
        let mut queue: Vec<String> = graph
            .values()
            .flatten()
            .filter(|(_, test)| *test)
            .map(|(file, _)| file.clone())
            .collect();
        while let Some(file) = queue.pop() {
            if tests.insert(file.clone()) {
                queue.extend(
                    graph
                        .get(&file)
                        .into_iter()
                        .flatten()
                        .map(|(f, _)| f.clone()),
                )
            }
        }
        let mut live = BTreeSet::new();
        let mut queue: Vec<_> = parsed
            .keys()
            .filter(|f| !tests.contains(*f))
            .cloned()
            .collect();
        while let Some(file) = queue.pop() {
            if live.insert(file.clone()) {
                queue.extend(
                    graph
                        .get(&file)
                        .into_iter()
                        .flatten()
                        .filter(|(_, test)| !*test)
                        .map(|(f, _)| f.clone()),
                )
            }
        }
        let mut index = Self {
            functions: BTreeMap::new(),
            data: BTreeMap::new(),
            imports: BTreeMap::new(),
            files: 0,
            constants: BTreeMap::new(),
            globs: BTreeMap::new(),
            locals: BTreeMap::new(),
            ambiguous_locals: BTreeMap::new(),
        };
        for (file, ast) in parsed {
            if tests.contains(&file) && !live.contains(&file) {
                continue;
            }
            index.files += 1;
            let rel = file
                .split("/src/")
                .nth(1)
                .unwrap_or(&file)
                .trim_end_matches(".rs");
            let module = if rel == "lib" {
                String::new()
            } else if rel == "main" {
                "__binary".into()
            } else {
                rel.trim_end_matches("/mod").replace('/', "::")
            };
            index.collect(&module, &ast.items)?;
        }
        Ok(index)
    }
    fn collect(&mut self, module: &str, items: &[Item]) -> Result<(), String> {
        let uses = self.imports.entry(module.into()).or_default();
        let globs = self.globs.entry(module.into()).or_default();
        for item in items {
            if let Item::Use(u) = item {
                if !excluded(&u.attrs) {
                    imports(&u.tree, "", uses, globs)
                }
            }
        }
        for item in items {
            match item {
                Item::Mod(m) if !excluded(&m.attrs) => {
                    if let Some((_, items)) = &m.content {
                        self.collect(&join(module, &m.ident.to_string()), items)?
                    }
                }
                Item::Const(c) if !excluded(&c.attrs) => {
                    self.constants
                        .insert(join(module, &c.ident.to_string()), (*c.expr).clone());
                }
                Item::Static(c) if !excluded(&c.attrs) => {
                    self.constants
                        .insert(join(module, &c.ident.to_string()), (*c.expr).clone());
                }
                Item::Struct(s) if !excluded(&s.attrs) => {
                    self.data
                        .insert(join(module, &s.ident.to_string()), Data::Struct(s.clone()));
                }
                Item::Enum(e) if !excluded(&e.attrs) => {
                    self.data
                        .insert(join(module, &e.ident.to_string()), Data::Enum(e.clone()));
                }
                Item::Type(t) if !excluded(&t.attrs) => {
                    self.data.insert(
                        join(module, &t.ident.to_string()),
                        Data::Alias((*t.ty).clone()),
                    );
                }
                Item::Fn(f) if !excluded(&f.attrs) => {
                    self.add(module, None, f.sig.clone(), (*f.block).clone())?
                }
                Item::Impl(i) if !excluded(&i.attrs) => {
                    let owner = type_name(&i.self_ty).unwrap_or_default();
                    for item in &i.items {
                        if let syn::ImplItem::Fn(f) = item {
                            if !excluded(&f.attrs) {
                                self.add(
                                    module,
                                    Some(owner.clone()),
                                    f.sig.clone(),
                                    f.block.clone(),
                                )?
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn add(
        &mut self,
        module: &str,
        owner: Option<String>,
        sig: syn::Signature,
        block: syn::Block,
    ) -> Result<(), String> {
        let key = join(
            module,
            &format!(
                "{}{}",
                owner.as_ref().map(|o| format!("{o}::")).unwrap_or_default(),
                sig.ident
            ),
        );
        // Trait methods with identical names are not silently picked by order.
        if self.functions.contains_key(&key) {
            self.functions.remove(&key);
            return Ok(());
        }
        struct LocalTypes {
            rows: BTreeMap<String, Data>,
            ambiguous: BTreeSet<String>,
        }
        impl<'ast> syn::visit::Visit<'ast> for LocalTypes {
            fn visit_item(&mut self, item: &'ast Item) {
                let data = match item {
                    Item::Struct(s) if !excluded(&s.attrs) => {
                        Some((s.ident.to_string(), Data::Struct(s.clone())))
                    }
                    Item::Enum(e) if !excluded(&e.attrs) => {
                        Some((e.ident.to_string(), Data::Enum(e.clone())))
                    }
                    Item::Type(t) if !excluded(&t.attrs) => {
                        Some((t.ident.to_string(), Data::Alias((*t.ty).clone())))
                    }
                    _ => None,
                };
                if let Some((name, data)) = data {
                    if self.rows.contains_key(&name) {
                        self.ambiguous.insert(name.clone());
                        self.rows.remove(&name);
                    } else if !self.ambiguous.contains(&name) {
                        self.rows.insert(name, data);
                    }
                }
            }
        }
        let mut locals = LocalTypes {
            rows: BTreeMap::new(),
            ambiguous: BTreeSet::new(),
        };
        syn::visit::Visit::visit_block(&mut locals, &block);
        self.locals.insert(key.clone(), locals.rows);
        self.ambiguous_locals.insert(key.clone(), locals.ambiguous);
        self.functions.insert(
            key.clone(),
            Function {
                key,
                module: module.into(),
                owner,
                sig,
                block,
            },
        );
        Ok(())
    }
    pub(crate) fn resolve(&self, module: &str, name: &str) -> String {
        self.resolve_depth(module, name, 0)
    }
    fn resolve_depth(&self, module: &str, name: &str, depth: usize) -> String {
        if depth > 32 {
            return format!("unresolved::{module}::{name}");
        }
        let name = name.trim_start_matches("::");
        if let Some(rest) = name.strip_prefix("crate::") {
            if self.data.contains_key(rest)
                || self.functions.contains_key(rest)
                || self.constants.contains_key(rest)
            {
                return rest.into();
            }
            return self.resolve_depth("", rest, depth + 1);
        }
        if let Some(rest) = name.strip_prefix("self::") {
            return join(module, rest);
        }
        if let Some(rest) = name.strip_prefix("super::") {
            let parent = module.rsplit_once("::").map(|(a, _)| a).unwrap_or("");
            return self.resolve_depth(parent, rest, depth + 1);
        }
        let mut parts = name.splitn(2, "::");
        let first = parts.next().unwrap_or("");
        if let Some(import) = self.imports.get(module).and_then(|m| m.get(first)) {
            let full = format!(
                "{import}{}",
                parts.next().map(|p| format!("::{p}")).unwrap_or_default()
            );
            if full != name {
                return self.resolve_depth(module, &full, depth + 1);
            }
        }
        let local = join(module, name);
        if self.data.contains_key(&local)
            || self.functions.contains_key(&local)
            || self.constants.contains_key(&local)
        {
            return local;
        }
        for glob in self.globs.get(module).into_iter().flatten() {
            let mut parts: Vec<_> = module
                .split("::")
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            for segment in glob.split("::") {
                match segment {
                    "crate" => parts.clear(),
                    "super" => {
                        parts.pop();
                    }
                    "self" => {}
                    s => parts.push(s.into()),
                }
            }
            let candidate = join(&parts.join("::"), name);
            if self.data.contains_key(&candidate)
                || self.functions.contains_key(&candidate)
                || self.constants.contains_key(&candidate)
            {
                return candidate;
            }
        }
        let suffix = format!("::{name}");
        let mut found = self
            .data
            .keys()
            .chain(self.functions.keys())
            .chain(self.constants.keys())
            .filter(|k| *k == name || k.ends_with(&suffix));
        match (found.next(), found.next()) {
            (Some(k), None) => k.clone(),
            _ => local,
        }
    }
}
pub(crate) fn join(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.into()
    } else {
        format!("{module}::{name}")
    }
}
pub(crate) fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Reference(r) => type_name(&r.elem),
        Type::Paren(p) => type_name(&p.elem),
        Type::Path(p) => {
            let last = p.path.segments.last()?;
            if matches!(
                last.ident.to_string().as_str(),
                "Arc" | "Rc" | "Box" | "Option" | "Result" | "Pin"
            ) {
                if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
                    if let Some(syn::GenericArgument::Type(t)) = args.args.first() {
                        return type_name(t);
                    }
                }
            }
            Some(
                p.path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::"),
            )
        }
        _ => None,
    }
}
