use crate::{
    Bound, Report,
    index::{Data, Function, Index, type_name},
};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Pat, Stmt, Type};
#[derive(Clone, Default)]
struct Value {
    raw: bool,
    map: bool,
    prefix: Vec<String>,
    hint: Option<String>,
    text: Option<String>,
    boolean: Option<bool>,
    items: Vec<Value>,
    known_items: bool,
    closure: Option<Box<syn::ExprClosure>>,
    captured: Option<Box<Env>>,
}
type Env = BTreeMap<String, Value>;
fn join_values(mut left: Value, right: Value) -> Value {
    if !left.raw && right.raw {
        left.prefix = right.prefix.clone();
    }
    left.raw |= right.raw;
    left.map |= right.map;
    left.known_items &= right.known_items;
    left.items.extend(right.items);
    if left.text != right.text {
        left.text = None;
    }
    if left.boolean != right.boolean {
        left.boolean = None;
    }
    if left.hint != right.hint {
        left.hint = None;
    }
    left
}
struct Trace<'a> {
    index: &'a Index,
    report: &'a mut Report,
    bound: Bound,
    module: String,
    owner: Option<String>,
    stack: Vec<String>,
    collect: bool,
    return_type: Option<Type>,
    returning: bool,
    scope_key: String,
    lexical_blocks: Vec<(String, syn::Block)>,
    next_block_id: usize,
}
pub(super) fn scan(index: &Index) -> Report {
    let mut report = Report {
        source_files: index.files,
        ..Default::default()
    };
    for function in index.functions.values() {
        let mut trace = Trace {
            index,
            report: &mut report,
            bound: Bound::default(),
            module: function.module.clone(),
            owner: function.owner.clone(),
            stack: Vec::new(),
            collect: true,
            return_type: None,
            returning: false,
            scope_key: function.key.clone(),
            lexical_blocks: Vec::new(),
            next_block_id: 0,
        };
        let mut env = Env::new();
        trace.arguments(function, &[], &mut env);
        trace.block(&function.block, &mut env);
    }
    report
}
fn path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|p| p.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
impl Trace<'_> {
    fn opaque(&mut self, prefix: &[String], reason: String) {
        let path = if prefix.is_empty() {
            "$".to_string()
        } else {
            prefix.join(".")
        };
        if !reason.contains(" Value map/array ") && !reason.contains(" Value adapter ") {
            self.bound.opaque_transforms.insert(path.clone());
        }
        self.bound.opaque.insert(reason);
        self.bound.opaque_paths.insert(path);
    }
    fn hint(&self, ty: &Type) -> Option<String> {
        type_name(ty).map(|name| {
            if name == "Self" {
                self.owner
                    .as_ref()
                    .map(|o| self.index.resolve(&self.module, o))
                    .unwrap_or(name)
            } else {
                self.index.resolve(&self.module, &name)
            }
        })
    }
    fn type_value(&self, ty: &Type) -> Value {
        match ty {
            Type::Tuple(t) => Value {
                items: t.elems.iter().map(|t| self.type_value(t)).collect(),
                ..Default::default()
            },
            Type::Reference(r) => self.type_value(&r.elem),
            Type::Path(p) => {
                if let Some(last) = p.path.segments.last() {
                    if matches!(
                        last.ident.to_string().as_str(),
                        "Arc" | "Rc" | "Box" | "Option" | "Result" | "Pin"
                    ) {
                        if let syn::PathArguments::AngleBracketed(a) = &last.arguments {
                            if let Some(syn::GenericArgument::Type(t)) = a.args.first() {
                                return self.type_value(t);
                            }
                        }
                    }
                }
                Value {
                    hint: self.hint(ty),
                    ..Default::default()
                }
            }
            _ => Value {
                hint: self.hint(ty),
                ..Default::default()
            },
        }
    }
    fn arguments(&self, function: &Function, values: &[Value], env: &mut Env) {
        let mut position = 0;
        for arg in &function.sig.inputs {
            match arg {
                syn::FnArg::Receiver(_) => {
                    env.insert(
                        "self".into(),
                        Value {
                            hint: function
                                .owner
                                .as_ref()
                                .map(|o| self.index.resolve(&function.module, o)),
                            ..Default::default()
                        },
                    );
                }
                syn::FnArg::Typed(p) => {
                    let mut value = values.get(position).cloned().unwrap_or_default();
                    position += 1;
                    if value.hint.is_none() {
                        let typed = self.type_value(&p.ty);
                        value.hint = typed.hint;
                        if value.items.is_empty() {
                            value.items = typed.items
                        }
                    }
                    Self::bind(&p.pat, value, env);
                }
            }
        }
    }
    fn bind(pattern: &Pat, value: Value, env: &mut Env) {
        match pattern {
            Pat::Ident(p) => {
                env.insert(p.ident.to_string(), value);
            }
            Pat::Type(p) => Self::bind(&p.pat, value, env),
            Pat::Reference(p) => Self::bind(&p.pat, value, env),
            Pat::Tuple(p) => {
                for (i, p) in p.elems.iter().enumerate() {
                    Self::bind(
                        p,
                        value.items.get(i).cloned().unwrap_or_else(|| value.clone()),
                        env,
                    )
                }
            }
            Pat::TupleStruct(p) => {
                let scalar = p.path.segments.len() > 1
                    && p.path
                        .segments
                        .iter()
                        .nth_back(1)
                        .is_some_and(|s| s.ident == "Value")
                    && p.path.segments.last().is_some_and(|s| {
                        matches!(s.ident.to_string().as_str(), "String" | "Bool" | "Number")
                    });
                for (i, p) in p.elems.iter().enumerate() {
                    let mut child = value.items.get(i).cloned().unwrap_or_else(|| value.clone());
                    if scalar {
                        child.raw = false
                    }
                    Self::bind(p, child, env)
                }
            }
            _ => {}
        }
    }
    fn block(&mut self, block: &syn::Block, env: &mut Env) -> Value {
        let key = format!("{}::block{}", self.scope_key, self.next_block_id);
        self.next_block_id += 1;
        self.lexical_blocks.push((key, block.clone()));
        let mut last = Value::default();
        for statement in &block.stmts {
            match statement {
                Stmt::Local(local) => {
                    if let Some(init) = &local.init {
                        let expected = if let Pat::Type(p) = &local.pat {
                            Some(&*p.ty)
                        } else {
                            None
                        };
                        let value = self.expr(&init.expr, env, expected);
                        Self::bind(&local.pat, value, env);
                        if let Some((_, other)) = &init.diverge {
                            let previous = self.returning;
                            self.expr(other, &mut env.clone(), None);
                            self.returning = previous;
                        }
                    }
                }
                Stmt::Expr(expr, _) => {
                    let expected = self.return_type.clone();
                    last = self.expr(expr, env, expected.as_ref())
                }
                Stmt::Macro(m) => {
                    self.tokens(&m.mac.tokens, env);
                }
                Stmt::Item(_) => {}
            }
            if self.returning {
                break;
            }
        }
        self.lexical_blocks.pop();
        last
    }
    fn field(&mut self, source: &Value, key: &str) -> Value {
        let mut value = source.clone();
        value
            .prefix
            .extend(key.split('/').filter(|s| !s.is_empty()).map(str::to_owned));
        if source.raw {
            self.bound.fields.insert(value.prefix.join("."));
        }
        value.text = None;
        value.boolean = None;
        value.hint = None;
        value.map = false;
        value.items.clear();
        value
    }
    fn function(&self, name: &str, receiver: Option<&Value>) -> Option<Function> {
        if let Some(receiver) = receiver {
            let hint = receiver.hint.as_ref()?;
            let found: Vec<_> = self
                .index
                .functions
                .values()
                .filter(|f| {
                    f.sig.ident == name
                        && f.owner
                            .as_ref()
                            .is_some_and(|o| self.index.resolve(&f.module, o) == *hint)
                })
                .collect();
            return if found.len() == 1 {
                Some(found[0].clone())
            } else {
                None
            };
        }
        if !name.contains("::") {
            for (scope, block) in self.lexical_blocks.iter().rev() {
                let local: Vec<_> = block
                    .stmts
                    .iter()
                    .filter_map(|s| match s {
                        Stmt::Item(syn::Item::Fn(f))
                            if f.sig.ident == name && !crate::index::excluded(&f.attrs) =>
                        {
                            Some(f)
                        }
                        _ => None,
                    })
                    .collect();
                if local.len() > 1 {
                    return None;
                }
                if let Some(function) = local.first() {
                    let depth = self
                        .lexical_blocks
                        .iter()
                        .position(|(key, _)| key == scope)
                        .unwrap()
                        + 1;
                    return Some(Function {
                        key: format!("{scope}::local::{name}"),
                        module: self.module.clone(),
                        owner: None,
                        sig: function.sig.clone(),
                        block: (*function.block).clone(),
                        lexical_blocks: self.lexical_blocks[..depth].to_vec(),
                    });
                }
            }
        }
        let name = if let Some(rest) = name.strip_prefix("Self::") {
            self.owner
                .as_ref()
                .map(|o| format!("{o}::{rest}"))
                .unwrap_or_else(|| name.into())
        } else {
            name.into()
        };
        let key = self.index.resolve(&self.module, &name);
        if let Some(f) = self.index.functions.get(&key) {
            return Some(f.clone());
        }
        if let Some((owner, method)) = name.rsplit_once("::") {
            let owner = self.index.resolve(&self.module, owner);
            return self.function(
                method,
                Some(&Value {
                    hint: Some(owner),
                    ..Default::default()
                }),
            );
        }
        None
    }
    fn invoke(&mut self, function: &Function, args: &[Value]) -> Value {
        let old_lexical =
            std::mem::replace(&mut self.lexical_blocks, function.lexical_blocks.clone());
        let old_module = self.module.clone();
        let old_owner = self.owner.clone();
        let old_scope = self.scope_key.clone();
        self.scope_key = function.key.clone();
        let old_return = self.return_type.clone();
        let old_returning = self.returning;
        self.returning = false;
        self.return_type = match &function.sig.output {
            syn::ReturnType::Type(_, t) => Some((**t).clone()),
            _ => None,
        };
        self.module = function.module.clone();
        self.owner = function.owner.clone();
        let output = match &function.sig.output {
            syn::ReturnType::Type(_, ty) => self.type_value(ty),
            _ => Value::default(),
        };
        if !args.iter().any(|v| v.raw) {
            self.module = old_module;
            self.owner = old_owner;
            self.scope_key = old_scope;
            self.return_type = old_return;
            self.returning = old_returning;
            self.lexical_blocks = old_lexical;
            return output;
        }
        if self.stack.contains(&function.key) {
            for arg in args.iter().filter(|v| v.raw) {
                self.opaque(
                    &arg.prefix,
                    format!("recursive caller transform {}", function.key),
                );
            }
            self.module = old_module;
            self.owner = old_owner;
            self.scope_key = old_scope;
            self.return_type = old_return;
            self.returning = old_returning;
            self.lexical_blocks = old_lexical;
            return args.iter().find(|v| v.raw).cloned().unwrap_or(output);
        }
        if self.stack.len() >= 8 {
            self.bound
                .unresolved
                .insert(format!("bounded/recursive helper {}", function.key));
            self.module = old_module;
            self.owner = old_owner;
            self.scope_key = old_scope;
            self.return_type = old_return;
            self.returning = old_returning;
            self.lexical_blocks = old_lexical;
            return Value::default();
        }
        self.bound.sources.insert(function.key.clone());
        self.stack.push(function.key.clone());
        let mut env = Env::new();
        self.arguments(function, args, &mut env);
        let mut result = self.block(&function.block, &mut env);
        if result.hint.is_none() {
            result.hint = output.hint;
            if result.items.is_empty() {
                result.items = output.items
            }
        }
        self.stack.pop();
        self.module = old_module;
        self.owner = old_owner;
        self.scope_key = old_scope;
        self.return_type = old_return;
        self.returning = old_returning;
        self.lexical_blocks = old_lexical;
        result
    }
    fn register(&mut self, name: Value, handler: &Expr, env: &Env) {
        let Some(name) = name.text else {
            self.report
                .unresolved_registrations
                .insert(format!("{} dynamic handler name", self.module));
            return;
        };
        let callback = match handler {
            Expr::Closure(c) => Some(c.clone()),
            Expr::Path(p) => env
                .get(&path_name(&p.path))
                .and_then(|v| v.closure.as_deref())
                .cloned(),
            _ => None,
        };
        let mut child = Trace {
            index: self.index,
            report: self.report,
            bound: Bound::default(),
            module: self.module.clone(),
            owner: self.owner.clone(),
            stack: Vec::new(),
            collect: false,
            return_type: None,
            returning: false,
            scope_key: self.scope_key.clone(),
            lexical_blocks: self.lexical_blocks.clone(),
            next_block_id: self.next_block_id,
        };
        if let Some(callback) = callback {
            let mut env = env.clone();
            for (i, param) in callback.inputs.iter().enumerate() {
                Self::bind(
                    param,
                    Value {
                        raw: i == 1,
                        ..Default::default()
                    },
                    &mut env,
                );
            }
            child.expr(&callback.body, &mut env, None);
        } else {
            child
                .bound
                .unresolved
                .insert(format!("{} unresolved callback", self.module));
        }
        child.bound.sources.insert(self.module.clone());
        let bound = child.bound;
        self.report.methods.entry(name).or_default().merge(bound);
    }
    fn expr(&mut self, expr: &Expr, env: &mut Env, expected: Option<&Type>) -> Value {
        match expr {
            Expr::Path(p) => {
                let name = path_name(&p.path);
                if let Some(value) = env.get(&name) {
                    value.clone()
                } else if let Some(value) = self
                    .index
                    .constants
                    .get(&self.index.resolve(&self.module, &name))
                    .cloned()
                {
                    self.expr(&value, env, expected)
                } else {
                    Value::default()
                }
            }
            Expr::Lit(l) => match &l.lit {
                syn::Lit::Str(s) => Value {
                    text: Some(s.value()),
                    ..Default::default()
                },
                syn::Lit::Bool(b) => Value {
                    boolean: Some(b.value),
                    ..Default::default()
                },
                _ => Value::default(),
            },
            Expr::Reference(r) => self.expr(&r.expr, env, expected),
            Expr::Paren(p) => self.expr(&p.expr, env, expected),
            Expr::Group(g) => self.expr(&g.expr, env, expected),
            Expr::Try(t) => self.expr(&t.expr, env, expected),
            Expr::Await(a) => self.expr(&a.base, env, expected),
            Expr::Async(a) => {
                let previous = self.returning;
                self.returning = false;
                let value = self.block(&a.block, env);
                self.returning = previous;
                value
            }
            Expr::Block(b) => self.block(&b.block, env),
            Expr::Unsafe(b) => self.block(&b.block, env),
            Expr::Closure(c) => {
                if c.inputs.is_empty() {
                    let previous = self.returning;
                    self.returning = false;
                    self.expr(&c.body, &mut env.clone(), None);
                    self.returning = previous;
                }
                Value {
                    closure: Some(Box::new(c.clone())),
                    captured: Some(Box::new(env.clone())),
                    ..Default::default()
                }
            }
            Expr::Array(a) => Value {
                known_items: true,
                items: a.elems.iter().map(|e| self.expr(e, env, None)).collect(),
                ..Default::default()
            },
            Expr::Tuple(a) => Value {
                items: a.elems.iter().map(|e| self.expr(e, env, None)).collect(),
                ..Default::default()
            },
            Expr::Struct(s) => {
                for field in &s.fields {
                    let value = self.expr(&field.expr, env, None);
                    if value.raw {
                        self.opaque(
                            &value.prefix,
                            format!(
                                "{} caller payload embedded in constructed struct",
                                self.module
                            ),
                        );
                    }
                }
                Value {
                    hint: Some(if s.path.is_ident("Self") {
                        self.owner
                            .as_ref()
                            .map(|o| self.index.resolve(&self.module, o))
                            .unwrap_or_default()
                    } else {
                        self.index.resolve(&self.module, &path_name(&s.path))
                    }),
                    ..Default::default()
                }
            }
            Expr::ForLoop(f) => {
                let sequence = self.expr(&f.expr, env, None);
                let items = if sequence.map {
                    vec![Value {
                        items: vec![
                            Value::default(),
                            Value {
                                map: false,
                                ..sequence.clone()
                            },
                        ],
                        ..Default::default()
                    }]
                } else if sequence.items.is_empty() && !sequence.known_items {
                    vec![sequence.clone()]
                } else {
                    sequence.items
                };
                for value in items {
                    let mut nested = env.clone();
                    Self::bind(&f.pat, value, &mut nested);
                    self.block(&f.body, &mut nested);
                }
                Value::default()
            }
            Expr::If(i) => {
                let value = self.expr(&i.cond, env, None);
                let previous = self.returning;
                let mut yes = Value::default();
                let mut no = Value::default();
                let mut yes_exit = false;
                let mut no_exit = false;
                if value.boolean != Some(false) {
                    self.returning = false;
                    let mut nested = env.clone();
                    if let Expr::Let(l) = &*i.cond {
                        let v = self.expr(&l.expr, env, None);
                        Self::bind(&l.pat, v, &mut nested);
                    }
                    yes = self.block(&i.then_branch, &mut nested);
                    yes_exit = self.returning;
                }
                if value.boolean != Some(true) {
                    self.returning = false;
                    if let Some((_, other)) = &i.else_branch {
                        no = self.expr(other, &mut env.clone(), expected);
                        no_exit = self.returning;
                    }
                }
                self.returning = previous
                    || match value.boolean {
                        Some(true) => yes_exit,
                        Some(false) => no_exit,
                        None => yes_exit && no_exit,
                    };
                match value.boolean {
                    Some(true) => yes,
                    Some(false) => no,
                    None => join_values(yes, no),
                }
            }
            Expr::Match(m) => {
                let source = self.expr(&m.expr, env, None);
                let previous = self.returning;
                let mut result = Value::default();
                let mut exits = Vec::new();
                let mut results = Vec::new();
                for arm in &m.arms {
                    let matched = pattern_matches(&arm.pat, source.text.as_deref());
                    if matched == Some(false) {
                        continue;
                    }
                    self.returning = false;
                    let mut nested = env.clone();
                    Self::bind(&arm.pat, source.clone(), &mut nested);
                    let condition = arm
                        .guard
                        .as_ref()
                        .and_then(|(_, e)| self.expr(e, &mut nested, None).boolean);
                    if condition == Some(false) {
                        continue;
                    }
                    result = self.expr(&arm.body, &mut nested, expected);
                    results.push(result.clone());
                    exits.push(self.returning);
                    if matched == Some(true) && arm.guard.is_none() {
                        break;
                    }
                }
                self.returning = previous || (!exits.is_empty() && exits.iter().all(|e| *e));
                results.into_iter().reduce(join_values).unwrap_or(result)
            }
            Expr::Let(l) => self.expr(&l.expr, env, None),
            Expr::Unary(u) => {
                let mut value = self.expr(&u.expr, env, None);
                if matches!(u.op, syn::UnOp::Not(_)) {
                    value.boolean = value.boolean.map(|b| !b);
                }
                value
            }
            Expr::Binary(b) => {
                let left = self.expr(&b.left, env, None);
                let right = self.expr(&b.right, env, None);
                let boolean = match &b.op {
                    syn::BinOp::Eq(_) => left.text.zip(right.text).map(|(a, b)| a == b),
                    syn::BinOp::Ne(_) => left.text.zip(right.text).map(|(a, b)| a != b),
                    syn::BinOp::And(_) => {
                        if left.boolean == Some(false) || right.boolean == Some(false) {
                            Some(false)
                        } else {
                            left.boolean.zip(right.boolean).map(|(a, b)| a && b)
                        }
                    }
                    syn::BinOp::Or(_) => {
                        if left.boolean == Some(true) || right.boolean == Some(true) {
                            Some(true)
                        } else {
                            left.boolean.zip(right.boolean).map(|(a, b)| a || b)
                        }
                    }
                    _ => None,
                };
                Value {
                    boolean,
                    ..Default::default()
                }
            }
            Expr::Index(i) => {
                let source = self.expr(&i.expr, env, None);
                let key = self.expr(&i.index, env, None);
                if let Some(key) = key.text {
                    self.field(&source, &key)
                } else {
                    if source.raw
                        && !matches!(&*i.index,Expr::Lit(l) if matches!(l.lit,syn::Lit::Int(_)))
                    {
                        self.bound.unresolved.insert(format!(
                            "{} dynamic index {}",
                            self.module,
                            source.prefix.join(".")
                        ));
                    }
                    source
                }
            }
            Expr::Field(f) => {
                let source = self.expr(&f.base, env, None);
                let name = match &f.member {
                    syn::Member::Named(n) => n.to_string(),
                    syn::Member::Unnamed(i) => {
                        if let Some(value) = source.items.get(i.index as usize) {
                            return value.clone();
                        }
                        let hint = source
                            .hint
                            .as_ref()
                            .and_then(|key| self.index.data.get(key))
                            .and_then(|data| match data {
                                Data::Struct(s) => s
                                    .fields
                                    .iter()
                                    .nth(i.index as usize)
                                    .and_then(|field| self.hint(&field.ty)),
                                _ => None,
                            });
                        return Value {
                            hint,
                            raw: source.raw,
                            prefix: source.prefix,
                            ..Default::default()
                        };
                    }
                };
                let field = source
                    .hint
                    .as_ref()
                    .and_then(|key| self.index.data.get(key).map(|d| (key.clone(), d)))
                    .and_then(|(key, data)| match data {
                        Data::Struct(s) => s
                            .fields
                            .iter()
                            .find(|field| field.ident.as_ref().is_some_and(|i| i == &name))
                            .cloned()
                            .map(|field| (key, s.attrs.clone(), field)),
                        _ => None,
                    });
                if let Some((key, attrs, field)) = field {
                    let previous = self.module.clone();
                    self.module = key
                        .rsplit_once("::")
                        .map(|(m, _)| m.to_owned())
                        .unwrap_or_default();
                    let mut typed = self.type_value(&field.ty);
                    self.module = previous;
                    let options = SerdeOptions::read(&field.attrs);
                    let parent = SerdeOptions::read(&attrs);
                    let scalar = typed.hint.as_ref().is_some_and(|h| {
                        matches!(
                            h.rsplit("::").next().unwrap_or(h),
                            "String"
                                | "str"
                                | "bool"
                                | "u8"
                                | "u16"
                                | "u32"
                                | "u64"
                                | "usize"
                                | "i8"
                                | "i16"
                                | "i32"
                                | "i64"
                                | "isize"
                                | "f32"
                                | "f64"
                        )
                    });
                    typed.raw = source.raw && !scalar && !options.skip;
                    typed.prefix = source.prefix;
                    if !options.flatten {
                        typed.prefix.push(
                            options
                                .rename
                                .unwrap_or_else(|| rename(&name, parent.rename_all.as_deref())),
                        )
                    }
                    return typed;
                }
                Value::default()
            }
            Expr::Assign(a) => {
                let value = self.expr(&a.right, env, None);
                if let Expr::Path(p) = &*a.left {
                    env.insert(path_name(&p.path), value.clone());
                }
                value
            }
            Expr::Return(r) => {
                let output = self.return_type.clone();
                let value = r
                    .expr
                    .as_ref()
                    .map(|e| self.expr(e, env, expected.or(output.as_ref())))
                    .unwrap_or_default();
                self.returning = true;
                value
            }
            Expr::Call(c) => {
                let name = match &*c.func {
                    Expr::Path(p) => path_name(&p.path),
                    _ => String::new(),
                };
                let args: Vec<_> = c.args.iter().map(|e| self.expr(e, env, None)).collect();
                if matches!(
                    name.as_str(),
                    "serde_json::to_vec"
                        | "serde_json::to_vec_pretty"
                        | "serde_json::to_value"
                        | "serde_json::to_string"
                        | "serde_json::to_string_pretty"
                        | "std::mem::take"
                ) {
                    if let Some(value) = args.iter().find(|v| v.raw) {
                        self.opaque(
                            &value.prefix,
                            format!("{} whole caller transform {name}", self.module),
                        );
                        return value.clone();
                    }
                    return Value::default();
                }
                if name == "serde_json::to_writer" {
                    for arg in args.iter().filter(|v| v.raw) {
                        self.opaque(
                            &arg.prefix,
                            format!("{} caller payload serialized to writer", self.module),
                        );
                    }
                    return Value::default();
                }
                if let Some(value) = env.get(&name).cloned() {
                    if let Some(callback) = value.closure {
                        let mut nested = value.captured.map(|e| *e).unwrap_or_else(|| env.clone());
                        for (p, arg) in callback.inputs.iter().zip(&args) {
                            Self::bind(p, arg.clone(), &mut nested)
                        }
                        let previous = self.returning;
                        self.returning = false;
                        let result = self.expr(&callback.body, &mut nested, expected);
                        self.returning = previous;
                        return result;
                    }
                }
                if name.ends_with("::from_value")
                    || name.ends_with("::from_str")
                    || name.ends_with("::from_slice")
                {
                    if args.first().is_some_and(|v| v.raw) {
                        let generic = match &*c.func {
                            Expr::Path(p) => {
                                p.path.segments.last().and_then(|p| match &p.arguments {
                                    syn::PathArguments::AngleBracketed(a) => {
                                        a.args.iter().find_map(|a| {
                                            if let syn::GenericArgument::Type(t) = a {
                                                Some(t)
                                            } else {
                                                None
                                            }
                                        })
                                    }
                                    _ => None,
                                })
                            }
                            _ => None,
                        };
                        if let Some(ty) = generic.or(expected) {
                            self.serde_type(ty, &args[0].prefix, &mut BTreeSet::new());
                            return Value {
                                raw: true,
                                prefix: args[0].prefix.clone(),
                                hint: self.hint(ty),
                                ..Default::default()
                            };
                        }
                        self.bound
                            .unresolved
                            .insert(format!("{} inferred serde target", self.module));
                    }
                    return Value::default();
                }
                if let Some(function) = self.function(&name, None) {
                    return self.invoke(&function, &args);
                }
                if name.ends_with("::new")
                    || name.ends_with("::open")
                    || name.ends_with("::default")
                {
                    let prefix = name.rsplit_once("::").map(|(p, _)| p).unwrap_or("");
                    if matches!(prefix, "Arc" | "std::sync::Arc" | "Box" | "Rc") {
                        return args.first().cloned().unwrap_or_default();
                    }
                    return Value {
                        hint: Some(self.index.resolve(&self.module, prefix)),
                        ..Default::default()
                    };
                }
                if self.index.locals.get(&self.scope_key).is_some_and(|types|matches!(types.get(&name),Some(Data::Struct(s)) if matches!(s.fields,syn::Fields::Unnamed(_)))){return Value{items:args,hint:Some(format!("{}::local::{name}",self.module)),..Default::default()}}
                if matches!(name.as_str(), "Some" | "Ok") {
                    return args.first().cloned().unwrap_or_default();
                }
                if args.iter().any(|v| v.raw) {
                    self.bound
                        .unresolved
                        .insert(format!("{} unresolved helper {name}", self.module));
                }
                Value::default()
            }
            Expr::MethodCall(c) => {
                let name = c.method.to_string();
                let transparent = matches!(
                    name.as_str(),
                    "unwrap"
                        | "unwrap_or"
                        | "unwrap_or_default"
                        | "unwrap_or_else"
                        | "expect"
                        | "context"
                        | "with_context"
                        | "map_err"
                );
                let receiver =
                    self.expr(&c.receiver, env, if transparent { expected } else { None });
                if name == "handler" && c.args.len() == 2 && self.collect {
                    let method = self.expr(&c.args[0], env, None);
                    self.register(method, &c.args[1], env);
                    return receiver;
                }
                let args: Vec<_> = c.args.iter().map(|a| self.expr(a, env, None)).collect();
                if matches!(
                    name.as_str(),
                    "expect"
                        | "map_err"
                        | "clone"
                        | "cloned"
                        | "to_owned"
                        | "as_ref"
                        | "as_mut"
                        | "unwrap"
                        | "unwrap_or"
                        | "unwrap_or_default"
                        | "unwrap_or_else"
                        | "ok_or"
                        | "ok_or_else"
                        | "context"
                        | "with_context"
                        | "iter"
                        | "iter_mut"
                        | "into_iter"
                        | "or_else"
                ) {
                    return receiver;
                }
                if name == "to_string" && receiver.raw {
                    self.opaque(
                        &receiver.prefix,
                        format!("{} caller value stringified", self.module),
                    );
                    return Value::default();
                }
                if name == "as_str" {
                    if receiver.raw && receiver.prefix.is_empty() {
                        self.opaque(
                            &receiver.prefix,
                            format!("{} whole caller scalar", self.module),
                        );
                    }
                    return Value {
                        text: receiver.text,
                        ..Default::default()
                    };
                }
                if name.starts_with("as_")
                    && !matches!(
                        name.as_str(),
                        "as_object" | "as_object_mut" | "as_array" | "as_array_mut"
                    )
                {
                    return Value::default();
                }
                if matches!(name.as_str(), "get" | "get_mut" | "pointer" | "remove") && receiver.raw
                {
                    if let Some(key) = args.first().and_then(|v| v.text.as_ref()) {
                        return self.field(&receiver, key);
                    }
                    if receiver.map {
                        self.opaque(
                            &receiver.prefix,
                            format!("{} dynamic map lookup", self.module),
                        );
                    } else {
                        self.bound.unresolved.insert(format!(
                            "{} dynamic field {}",
                            self.module,
                            receiver.prefix.join(".")
                        ));
                    }
                    return receiver;
                }
                if matches!(
                    name.as_str(),
                    "as_object" | "as_object_mut" | "as_array" | "as_array_mut"
                ) {
                    if receiver.raw {
                        self.opaque(
                            &receiver.prefix,
                            format!(
                                "{} Value map/array {}",
                                self.module,
                                receiver.prefix.join(".")
                            ),
                        );
                    }
                    let mut result = receiver.clone();
                    if matches!(name.as_str(), "as_object" | "as_object_mut") {
                        result.map = true;
                    }
                    return result;
                }
                if matches!(name.as_str(), "values" | "values_mut") && receiver.raw && receiver.map
                {
                    self.opaque(
                        &receiver.prefix,
                        format!("{} caller map values inspected", self.module),
                    );
                    return Value::default();
                }
                if name == "keys" && receiver.raw && receiver.map {
                    self.bound
                        .key_inspections
                        .insert(if receiver.prefix.is_empty() {
                            "$".into()
                        } else {
                            receiver.prefix.join(".")
                        });
                    return Value::default();
                }
                if name == "write_all" && args.iter().any(|v| v.raw) {
                    for arg in args.iter().filter(|v| v.raw) {
                        self.opaque(
                            &arg.prefix,
                            format!("{} caller bytes written to sink", self.module),
                        );
                    }
                    return Value::default();
                }
                if matches!(
                    name.as_str(),
                    "map"
                        | "and_then"
                        | "filter_map"
                        | "flat_map"
                        | "is_some_and"
                        | "is_none_or"
                        | "filter"
                        | "sort_by"
                ) {
                    if let Some(Expr::Path(path)) = c.args.first() {
                        let callback = path_name(&path.path);
                        if let Some(function) = self.function(&callback, None) {
                            return self.invoke(&function, &[receiver.clone()]);
                        }
                        if matches!(
                            callback.as_str(),
                            "Vec::is_empty"
                                | "str::is_empty"
                                | "String::is_empty"
                                | "Value::is_null"
                                | "Value::is_string"
                                | "Value::is_object"
                                | "Value::is_array"
                                | "Value::is_number"
                                | "Value::is_boolean"
                        ) {
                            return Value::default();
                        }
                        if callback.starts_with("Value::as_")
                            || callback.starts_with("serde_json::Value::as_")
                        {
                            if callback.ends_with("as_object") || callback.ends_with("as_array") {
                                self.opaque(
                                    &receiver.prefix,
                                    format!("{} Value adapter {callback}", self.module),
                                );
                                let mut result = receiver;
                                result.map = callback.ends_with("as_object");
                                return result;
                            }
                            return Value::default();
                        }
                        if matches!(
                            callback.as_str(),
                            "str::to_owned" | "str::to_string" | "String::from"
                        ) {
                            return Value::default();
                        }
                    }
                    if let Some(value) = args.first() {
                        if let Some(callback) = value.closure.as_deref() {
                            let mut nested = value
                                .captured
                                .as_deref()
                                .cloned()
                                .unwrap_or_else(|| env.clone());
                            for p in &callback.inputs {
                                Self::bind(p, receiver.clone(), &mut nested)
                            }
                            let previous = self.returning;
                            self.returning = false;
                            let result = self.expr(&callback.body, &mut nested, expected);
                            self.returning = previous;
                            return result;
                        }
                    }
                }
                if name == "starts_with" || name == "ends_with" {
                    return Value {
                        boolean: receiver
                            .text
                            .as_ref()
                            .zip(args.first().and_then(|v| v.text.as_ref()))
                            .map(|(a, b)| {
                                if name == "starts_with" {
                                    a.starts_with(b)
                                } else {
                                    a.ends_with(b)
                                }
                            }),
                        ..Default::default()
                    };
                }
                if matches!(name.as_str(), "push" | "insert" | "contains_key")
                    && args.iter().any(|v| v.raw)
                {
                    for arg in args.iter().filter(|v| v.raw) {
                        self.opaque(
                            &arg.prefix,
                            format!("{} caller value stored by {name}", self.module),
                        );
                    }
                    return Value::default();
                }
                if let Some(function) = self.function(&name, Some(&receiver)) {
                    return self.invoke(&function, &args);
                }
                if receiver.raw
                    && !matches!(
                        name.as_str(),
                        "is_null"
                            | "is_object"
                            | "is_array"
                            | "is_string"
                            | "is_number"
                            | "is_boolean"
                            | "is_i64"
                            | "is_u64"
                            | "is_f64"
                            | "is_some"
                            | "is_none"
                            | "is_ok"
                            | "is_err"
                            | "is_empty"
                    )
                    && !matches!(
                        name.as_str(),
                        "len"
                            | "keys"
                            | "values"
                            | "values_mut"
                            | "contains_key"
                            | "contains"
                            | "collect"
                            | "flatten"
                            | "count"
                            | "all"
                            | "any"
                            | "map_or"
                            | "map_or_else"
                    )
                    || args.iter().any(|v| v.raw)
                {
                    self.bound.unresolved.insert(format!(
                        "{} unresolved receiver {}.{name}",
                        self.module,
                        receiver.hint.unwrap_or_default()
                    ));
                }
                Value::default()
            }
            Expr::Macro(m) => {
                self.tokens(&m.mac.tokens, env);
                Value::default()
            }
            _ => {
                struct Children<'a, 'b, 'c> {
                    trace: &'a mut Trace<'b>,
                    env: &'c mut Env,
                }
                impl<'ast> syn::visit::Visit<'ast> for Children<'_, '_, '_> {
                    fn visit_expr(&mut self, e: &'ast Expr) {
                        self.trace.expr(e, self.env, None);
                    }
                }
                syn::visit::visit_expr(&mut Children { trace: self, env }, expr);
                Value::default()
            }
        }
    }
    fn tokens(&mut self, tokens: &proc_macro2::TokenStream, env: &mut Env) {
        use syn::parse::Parser;
        if let Ok(expressions) =
            syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated
                .parse2(tokens.clone())
        {
            for expr in expressions {
                self.expr(&expr, env, None);
            }
            return;
        }
        // JSON object syntax is not a Rust block. Parse its value expressions,
        // never attribute literal outbound object keys to the caller.
        let mut parts = Vec::new();
        let mut current = proc_macro2::TokenStream::new();
        for token in tokens.clone() {
            if matches!(&token,proc_macro2::TokenTree::Punct(p) if p.as_char()==',') {
                parts.push(current);
                current = proc_macro2::TokenStream::new()
            } else {
                current.extend([token]);
            }
        }
        parts.push(current);
        for part in parts {
            let mut value = proc_macro2::TokenStream::new();
            let mut colon = false;
            for token in part.clone() {
                if !colon && matches!(&token,proc_macro2::TokenTree::Punct(p) if p.as_char()==':') {
                    colon = true;
                    continue;
                }
                if colon {
                    value.extend([token]);
                }
            }
            if colon {
                self.tokens(&value, env)
            } else {
                for token in part {
                    if let proc_macro2::TokenTree::Group(group) = token {
                        self.tokens(&group.stream(), env)
                    }
                }
            }
        }
    }
    fn serde_type(&mut self, ty: &Type, prefix: &[String], seen: &mut BTreeSet<String>) {
        match ty {
            Type::Reference(r) => return self.serde_type(&r.elem, prefix, seen),
            Type::Paren(p) => return self.serde_type(&p.elem, prefix, seen),
            Type::Array(a) => return self.serde_type(&a.elem, prefix, seen),
            Type::Slice(a) => return self.serde_type(&a.elem, prefix, seen),
            Type::Tuple(t) => {
                for ty in &t.elems {
                    self.serde_type(ty, prefix, seen)
                }
                return;
            }
            _ => {}
        }
        let Type::Path(path) = ty else {
            self.bound
                .unresolved
                .insert(format!("{} unsupported serde type", self.module));
            return;
        };
        let Some(last) = path.path.segments.last() else {
            return;
        };
        let name = last.ident.to_string();
        if matches!(
            name.as_str(),
            "Option" | "Result" | "Box" | "Vec" | "Arc" | "Rc" | "BTreeMap" | "HashMap"
        ) {
            if matches!(name.as_str(), "BTreeMap" | "HashMap") {
                self.opaque(
                    prefix,
                    format!("{} serde map {}", self.module, prefix.join(".")),
                );
            }
            if let syn::PathArguments::AngleBracketed(arguments) = &last.arguments {
                let types: Vec<_> = arguments
                    .args
                    .iter()
                    .filter_map(|a| {
                        if let syn::GenericArgument::Type(t) = a {
                            Some(t)
                        } else {
                            None
                        }
                    })
                    .collect();
                let skip = usize::from(matches!(name.as_str(), "BTreeMap" | "HashMap"));
                for ty in types.into_iter().skip(skip) {
                    self.serde_type(ty, prefix, seen)
                }
            }
            return;
        }
        if matches!(
            name.as_str(),
            "String"
                | "str"
                | "bool"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "usize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "isize"
                | "f32"
                | "f64"
        ) {
            if prefix.is_empty() {
                self.opaque(prefix, format!("{} whole scalar serde {name}", self.module));
            }
            return;
        }
        if name == "Value" {
            self.opaque(
                prefix,
                format!("{} serde Value {}", self.module, prefix.join(".")),
            );
            return;
        }
        if self
            .index
            .ambiguous_locals
            .get(&self.scope_key)
            .is_some_and(|names| names.contains(&name))
        {
            self.bound
                .unresolved
                .insert(format!("{} ambiguous local serde {name}", self.module));
            return;
        }
        let local = self
            .index
            .locals
            .get(&self.scope_key)
            .and_then(|types| types.get(&name))
            .cloned();
        let key = if local.is_some() {
            format!("{}::local::{name}", self.module)
        } else {
            self.index.resolve(&self.module, &path_name(&path.path))
        };
        if !seen.insert(key.clone()) {
            return;
        }
        let was_local = local.is_some();
        let Some(data) = local.or_else(|| self.index.data.get(&key).cloned()) else {
            self.bound
                .unresolved
                .insert(format!("{} unknown serde type {key}", self.module));
            return;
        };
        self.bound.sources.insert(key.clone());
        let previous = self.module.clone();
        if !was_local {
            self.module = key
                .rsplit_once("::")
                .map(|(m, _)| m.to_owned())
                .unwrap_or_default();
        }
        match data {
            Data::Alias(ty) => self.serde_type(&ty, prefix, seen),
            Data::Struct(s) => {
                let options = SerdeOptions::read(&s.attrs);
                if options.custom {
                    self.bound
                        .unresolved
                        .insert(format!("custom serde container {key}"));
                }
                self.serde_fields(&s.fields, &options, prefix, seen)
            }
            Data::Enum(e) => {
                let options = SerdeOptions::read(&e.attrs);
                if options.custom {
                    self.bound
                        .unresolved
                        .insert(format!("custom serde enum {key}"));
                }
                if let Some(tag) = &options.tag {
                    let mut path = prefix.to_vec();
                    path.push(tag.clone());
                    self.bound.fields.insert(path.join("."));
                }
                for variant in e.variants {
                    let variant_options = SerdeOptions::read(&variant.attrs);
                    if variant_options.skip {
                        continue;
                    }
                    let mut path = prefix.to_vec();
                    if let Some(content) = &options.content {
                        path.push(content.clone());
                        self.bound.fields.insert(path.join("."));
                    } else if options.tag.is_none() && !options.untagged {
                        let name = variant_options.rename.clone().unwrap_or_else(|| {
                            rename(&variant.ident.to_string(), options.rename_all.as_deref())
                        });
                        path.push(name);
                        self.bound.fields.insert(path.join("."));
                    }
                    self.serde_fields(&variant.fields, &variant_options, &path, seen);
                }
            }
        }
        self.module = previous;
        seen.remove(&key);
    }
    fn serde_fields(
        &mut self,
        fields: &syn::Fields,
        parent: &SerdeOptions,
        prefix: &[String],
        seen: &mut BTreeSet<String>,
    ) {
        for field in fields {
            let options = SerdeOptions::read(&field.attrs);
            if options.skip {
                continue;
            }
            if options.custom {
                self.bound
                    .unresolved
                    .insert(format!("{} custom serde field", self.module));
            }
            if options.flatten || parent.transparent {
                self.serde_type(&field.ty, prefix, seen);
                continue;
            }
            let Some(ident) = &field.ident else {
                self.serde_type(&field.ty, prefix, seen);
                continue;
            };
            let name = options.rename.clone().unwrap_or_else(|| {
                rename(
                    ident.to_string().trim_start_matches("r#"),
                    parent.rename_all.as_deref(),
                )
            });
            for name in std::iter::once(name).chain(options.aliases) {
                let mut path = prefix.to_vec();
                path.push(name);
                self.bound.fields.insert(path.join("."));
                self.serde_type(&field.ty, &path, seen);
            }
        }
    }
}
fn pattern_matches(pattern: &Pat, value: Option<&str>) -> Option<bool> {
    match pattern {
        Pat::Lit(p) => match &p.lit {
            syn::Lit::Str(s) => value.map(|v| v == s.value()),
            _ => None,
        },
        Pat::Or(p) => {
            let values: Vec<_> = p.cases.iter().map(|p| pattern_matches(p, value)).collect();
            if values.contains(&Some(true)) {
                Some(true)
            } else if values.iter().all(|v| *v == Some(false)) {
                Some(false)
            } else {
                None
            }
        }
        Pat::Wild(_) | Pat::Ident(_) => Some(true),
        _ => None,
    }
}
#[derive(Default)]
struct SerdeOptions {
    rename: Option<String>,
    rename_all: Option<String>,
    aliases: Vec<String>,
    skip: bool,
    flatten: bool,
    transparent: bool,
    untagged: bool,
    tag: Option<String>,
    content: Option<String>,
    custom: bool,
}
impl SerdeOptions {
    fn read(attrs: &[syn::Attribute]) -> Self {
        use syn::parse::Parser;
        let mut out = Self::default();
        for attr in attrs {
            if !attr.path().is_ident("serde") {
                continue;
            }
            let syn::Meta::List(list) = &attr.meta else {
                continue;
            };
            let Ok(entries) =
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
                    .parse2(list.tokens.clone())
            else {
                out.custom = true;
                continue;
            };
            for entry in entries {
                match entry {
                    syn::Meta::Path(p) => {
                        out.skip |= p.is_ident("skip") || p.is_ident("skip_deserializing");
                        out.flatten |= p.is_ident("flatten");
                        out.transparent |= p.is_ident("transparent");
                        out.untagged |= p.is_ident("untagged");
                    }
                    syn::Meta::NameValue(p) => {
                        let syn::Expr::Lit(value) = p.value else {
                            continue;
                        };
                        let syn::Lit::Str(value) = value.lit else {
                            continue;
                        };
                        let value = value.value();
                        if p.path.is_ident("rename") {
                            out.rename = Some(value)
                        } else if p.path.is_ident("rename_all") {
                            out.rename_all = Some(value)
                        } else if p.path.is_ident("alias") {
                            out.aliases.push(value)
                        } else if p.path.is_ident("tag") {
                            out.tag = Some(value)
                        } else if p.path.is_ident("content") {
                            out.content = Some(value)
                        } else if p.path.is_ident("with")
                            || p.path.is_ident("deserialize_with")
                            || p.path.is_ident("from")
                            || p.path.is_ident("try_from")
                        {
                            out.custom = true
                        }
                    }
                    syn::Meta::List(p) => {
                        if p.path.is_ident("rename") || p.path.is_ident("rename_all") {
                            if let Ok(entries) = syn::punctuated::Punctuated::<
                                syn::Meta,
                                syn::Token![,],
                            >::parse_terminated
                                .parse2(p.tokens)
                            {
                                for entry in entries {
                                    if let syn::Meta::NameValue(v) = entry {
                                        if v.path.is_ident("deserialize") {
                                            if let syn::Expr::Lit(l) = v.value {
                                                if let syn::Lit::Str(value) = l.lit {
                                                    if p.path.is_ident("rename") {
                                                        out.rename = Some(value.value())
                                                    } else {
                                                        out.rename_all = Some(value.value())
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }
}
fn rename(name: &str, rule: Option<&str>) -> String {
    match rule {
        Some("lowercase") => name.to_ascii_lowercase(),
        Some("UPPERCASE" | "SCREAMING_SNAKE_CASE") => name.to_ascii_uppercase(),
        Some("kebab-case") => name.replace('_', "-"),
        Some("SCREAMING-KEBAB-CASE") => name.to_ascii_uppercase().replace('_', "-"),
        Some("PascalCase" | "camelCase") => {
            let mut out = String::new();
            let mut upper = true;
            for c in name.chars() {
                if c == '_' {
                    upper = true;
                    continue;
                }
                out.push(if upper { c.to_ascii_uppercase() } else { c });
                upper = false;
            }
            if rule == Some("camelCase") {
                if let Some(first) = out.get_mut(..1) {
                    first.make_ascii_lowercase();
                }
            }
            out
        }
        _ => name.into(),
    }
}
