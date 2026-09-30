use std::collections::BTreeMap;
use workspacer_capability_source_check::scan;
fn fixture(body: &str) -> workspacer_capability_source_check::Report {
    scan(BTreeMap::from([("src/lib.rs".into(), body.into())])).unwrap()
}
#[test]
fn follows_actual_caller_helpers_and_distinguishes_response_decoding() {
    let report = fixture(
        r#"
    struct Request { #[serde(rename="bytesB64")] bytes: String, cwd: String }
    struct Response { command: String, env: String }
    fn read(raw: Value)->Result<Request> { serde_json::from_value(raw) }
    fn install(options:Options){options.handler("sessions.terminalInput",|_,params|async move {
       let request=read(params)?;
       let response:Response=serde_json::from_value(observed_response()).unwrap();
       Ok(request)
    });}
    "#,
    );
    let bound = &report.methods["sessions.terminalInput"];
    assert_eq!(
        bound.fields.iter().map(String::as_str).collect::<Vec<_>>(),
        ["bytesB64", "cwd"]
    );
    assert!(bound.unresolved.is_empty(), "{bound:?}");
}
#[test]
fn serde_named_nested_alias_skip_and_map_shapes_are_not_invisible() {
    let report = fixture(
        r#"
    #[serde(rename_all="camelCase")]
    struct Update { config_dir:String, extra_args:Vec<String>, #[serde(skip_deserializing)] shell:String,
      #[serde(skip_serializing,alias="exec")] command:String }
    struct Request { updates:Option<Box<Update>>, items:Vec<Update>, metadata:std::collections::BTreeMap<String,Update> }
    fn install(options:Options){options.handler("claude.profiles.update",|_,params|async move {
        let p:Request=serde_json::from_value(params).unwrap(); Ok(p)
    });}
    "#,
    );
    let bound = &report.methods["claude.profiles.update"];
    for name in [
        "updates.configDir",
        "updates.extraArgs",
        "updates.command",
        "updates.exec",
        "items.configDir",
        "metadata.configDir",
    ] {
        assert!(bound.fields.contains(name), "missing {name}: {bound:?}");
    }
    assert!(!bound.fields.iter().any(|n| n.ends_with("shell")));
    assert!(!bound.opaque.is_empty());
    assert!(bound.unresolved.is_empty(), "{bound:?}");
}
#[test]
fn typed_key_helper_map_indexes_and_whole_payload_remain_distinct() {
    let report = fixture(
        r#"
    fn typed(raw:&Value,key:&str)->Option<&str>{raw.get(key).and_then(Value::as_str)}
    fn install(options:Options){
      options.handler("terminals.create",|_,params|async move{let shell=typed(&params,"shell");let cwd=params["cwd"].as_str();Ok(shell)});
      options.handler("config.save",|_,params|async move{let partial:std::collections::BTreeMap<String,Value>=serde_json::from_value(params)?;Ok(partial)});
    }
    "#,
    );
    let terminal = &report.methods["terminals.create"];
    assert!(terminal.fields.contains("shell"));
    assert!(terminal.fields.contains("cwd"));
    assert!(terminal.opaque.is_empty());
    assert!(!report.methods["config.save"].opaque.is_empty());
}
#[test]
fn callback_capture_and_receiver_type_follow_owned_scheduled_helpers() {
    let report = fixture(
        r#"
    struct Service {} impl Service { fn call(&self,method:&str,raw:Value)->Result<Value>{ match method {"sessions.load"=>Ok(raw["filename"].clone()),_=>Ok(Value::Null)} } }
    fn install(options:Options){let service=Arc::new(Service{});for method in ["sessions.load"] {let service=service.clone(); options.handler(method,move|_,params|async move {tokio::task::spawn_blocking(move|| service.call(method,params)).await?});}}
    "#,
    );
    let bound = &report.methods["sessions.load"];
    assert!(bound.fields.contains("filename"), "{bound:?}");
    assert!(bound.unresolved.is_empty(), "{bound:?}");
}
#[test]
fn test_only_ancestry_and_response_or_literal_decoys_cannot_supply_evidence() {
    let source = r#"#[cfg(test)] #[path="audit.rs"] mod audit;
    fn install(options:Options){options.handler("fs.read",|_,params|async move{let text="params[\"shell\"]";Ok(params["path"].clone())});}"#;
    let report = scan(BTreeMap::from([
        ("src/lib.rs".into(), source.into()),
        (
            "src/audit.rs".into(),
            "#[path=\"nested.rs\"] mod nested;".into(),
        ),
        (
            "src/nested.rs".into(),
            "fn fixture(o:Options){o.handler(\"fake.actor\",|_,p|async{p[\"command\"]});}".into(),
        ),
    ]))
    .unwrap();
    assert!(!report.methods.contains_key("fake.actor"));
    assert_eq!(report.methods["fs.read"].fields.len(), 1);
}
#[test]
fn unsupported_serde_custom_helper_and_dynamic_keys_are_explicit_gaps() {
    let report = fixture(
        r#"
    #[serde(from="String")] struct Custom { command:String }
    fn install(o:Options){o.handler("unsafe.actor",|_,params|async move {
      let p:Custom=serde_json::from_value(params.clone())?;let x=params[unknown_key()]; external_helper(params);Ok(p)
    });}
    "#,
    );
    let gaps = &report.methods["unsafe.actor"].unresolved;
    assert!(gaps.iter().any(|e| e.contains("custom serde")), "{gaps:?}");
    assert!(gaps.iter().any(|e| e.contains("dynamic index")));
    assert!(gaps.iter().any(|e| e.contains("external_helper")));
}
#[test]
fn imported_options_fields_keep_receiver_types_in_if_let() {
    let report=scan(BTreeMap::from([
 ("src/lib.rs".into(),"pub use runtime::Options; mod runtime; mod services;".into()),
 ("src/main.rs".into(),"use external_crate::Options; fn main() {}".into()),
 ("src/runtime.rs".into(),"pub struct Options { pub workflow: Option<Arc<crate::services::Workflow>> }".into()),
 ("src/services.rs".into(),r#"use crate::Options;
 pub struct Workflow {} impl Workflow {fn request(&self,p:&Value){let _=p["cwd"].as_str();}}
 fn install(mut options:Options) {if let Some(runtime)=options.workflow.clone() {options=options.handler("desktop.request",move|_,p|async move {runtime.request(&p["request"])});}}"#.into())])).unwrap();
    let b = &report.methods["desktop.request"];
    assert!(b.fields.contains("request.cwd"), "{b:?}");
    assert!(b.unresolved.is_empty(), "{b:?}");
}
#[test]
fn unknown_input_receiver_and_nested_struct_escape_are_not_silent() {
    let report = fixture(
        r#"fn install(o:Options){o.handler("actor.call",|_,p|async move {
 p.external_transform();let wrapped=Envelope{value:p["nested"].clone()};Ok(wrapped)
 });}"#,
    );
    let b = &report.methods["actor.call"];
    assert!(
        b.unresolved
            .iter()
            .any(|s| s.contains("external_transform")),
        "{b:?}"
    );
    assert!(b.opaque_paths.contains("nested"), "{b:?}");
}
#[test]
fn unknown_match_branch_does_not_hide_mcp_and_map_iteration_keeps_values() {
    let report = fixture(
        r#"
 fn kind(v:&Value)->&str {match v.as_str(){Some("mcp")=>"mcp",_=>"other"}}
 fn install(o:Options){o.handler("library.save",|_,p|async move {
 if kind(&p["kind"])=="mcp" {let _=p["mcp"]["command"].as_str();}
 if let Some(map)=p["rates"].as_object(){for (key,value) in map {let _=value["path"].as_str();}}
 });}"#,
    );
    let b = &report.methods["library.save"];
    assert!(b.fields.contains("mcp.command"), "{b:?}");
    assert!(b.fields.contains("rates.path"), "{b:?}");
    assert!(b.opaque_paths.contains("rates"), "{b:?}");
}
#[test]
fn recursive_map_transforms_retain_explicit_opaque_evidence() {
    let report = fixture(
        r#"
 fn recurse(p:&Value){let _=p["command"].as_str();recurse(p);}
 fn install(o:Options){o.handler("config.save",|_,p|async move {recurse(&p);});}
 "#,
    );
    let b = &report.methods["config.save"];
    assert!(b.fields.contains("command"));
    assert!(b.opaque_paths.contains("$"));
    assert!(b.opaque.iter().any(|s| s.contains("recursive")));
}
#[test]
fn value_function_pointer_adapters_and_sort_comparators_keep_input_evidence() {
    let report = fixture(
        r#"
    fn install(o:Options){o.handler("actor.call",|_,p|async move {
      let child=p.get("nested").and_then(Value::as_object);
      if let Some(map)=child {let _=map.get("command").and_then(Value::as_str);}
      p["rows"].as_array().unwrap().sort_by(|a,b|a["path"].as_str().cmp(&b["path"].as_str()));
    });}
    "#,
    );
    let b = &report.methods["actor.call"];
    assert!(b.fields.contains("nested.command"), "{b:?}");
    assert!(b.fields.contains("rows.path"), "{b:?}");
    assert!(b.unresolved.is_empty(), "{b:?}");
}
#[test]
fn local_helpers_are_hoisted_lexically_and_never_leak_into_other_blocks() {
    let report = fixture(
        r#"
    fn read(p:&Value){let _=p["shell"].as_str();}
    fn install(o:Options){
      o.handler("first",|_,p|async move {
        local(&p,"cwd");
        fn local(p:&Value,key:&str){p.get(key).and_then(Value::as_str);}
        {fn read(p:&Value){let _=p["directory"].as_str();}read(&p);}
        read(&p);
      });
      o.handler("second",|_,p|async move {read(&p);});
    }
    "#,
    );
    let first = &report.methods["first"];
    assert_eq!(
        first.fields.iter().map(String::as_str).collect::<Vec<_>>(),
        ["cwd", "directory", "shell"]
    );
    assert!(first.unresolved.is_empty(), "{first:?}");
    let second = &report.methods["second"];
    assert_eq!(
        second.fields.iter().map(String::as_str).collect::<Vec<_>>(),
        ["shell"]
    );
}
#[test]
fn local_helpers_keep_declaration_scope_when_called_under_a_shadowing_function() {
    let report = fixture(
        r#"
    fn install(o:Options){o.handler("one",|_,p|async move {
      fn leaf(p:&Value){let _=p["cwd"].as_str();}
      fn outer(p:&Value){leaf(p);}
      {fn leaf(p:&Value){let _=p["command"].as_str();}outer(&p);}
    });}
    "#,
    );
    let b = &report.methods["one"];
    assert_eq!(
        b.fields.iter().map(String::as_str).collect::<Vec<_>>(),
        ["cwd"]
    );
    assert!(b.unresolved.is_empty(), "{b:?}");
}
#[test]
fn known_empty_key_arrays_do_not_read_unknown_fields_and_unknown_branches_keep_both_arrays() {
    let report = fixture(
        r#"
    fn install(o:Options){o.handler("one",|_,p|async move {
      let empty:&[&str]=&[];for key in empty {let _=p.get(*key);}
      let keys=if unknown() {&["cwd"]}else{&["shell"]};
      for key in keys {let _=p.get(*key).and_then(Value::as_str);}
      for key in p.as_object().into_iter().flat_map(|map|map.keys()){let _=key.chars();}
    });}
    "#,
    );
    let b = &report.methods["one"];
    assert_eq!(
        b.fields.iter().map(String::as_str).collect::<Vec<_>>(),
        ["cwd", "shell"]
    );
    assert!(b.unresolved.is_empty(), "{b:?}");
    assert!(b.key_inspections.contains("$"));
    assert!(b.opaque_transforms.is_empty());
}
#[test]
fn local_unknown_raw_sinks_remain_gaps_and_key_inspection_does_not_hide_serialization() {
    let report = fixture(
        r#"
    fn install(o:Options){o.handler("one",|_,p|async move {
      fn local(p:Value){p.external_transform();}
      for key in p.as_object().into_iter().flat_map(|map|map.keys()){let _=key;}
      serde_json::to_vec(&p);local(p);
    });}
    "#,
    );
    let b = &report.methods["one"];
    assert!(
        b.unresolved
            .iter()
            .any(|s| s.contains("external_transform")),
        "{b:?}"
    );
    assert!(b.key_inspections.contains("$"));
    assert!(b.opaque_transforms.contains("$"));
}

#[test]
fn method_selected_literal_lists_and_optional_keys_do_not_leak_into_other_methods() {
    let report = fixture(
        r#"
    fn validate(method:&str, params:&mut Value) {
        let texts:&[&str]=match method {"sessions.transcript"=>&["cwd"],"claude.answer"=>&["text"],_=>&[]};
        for key in texts {let _=params.get(*key);}
        let integer=match method {"sessions.conversation"=>Some("sinceSeq"),"claude.answer"=>Some("option"),_=>None};
        if let Some(key)=integer {let _=params.get(key);}
        if method=="claude.gate" && params.get("on").is_some() {}
    }
    fn install(options:Options) {for method in ["sessions.snapshot","sessions.transcript","sessions.conversation","claude.answer","claude.gate"] {
      options.handler(method,move|_,mut params|async move {validate(method,&mut params);Ok(())});
    }}
    "#,
    );
    for (method, expected) in [
        ("sessions.snapshot", vec![]),
        ("sessions.transcript", vec!["cwd"]),
        ("sessions.conversation", vec!["sinceSeq"]),
        ("claude.answer", vec!["option", "text"]),
        ("claude.gate", vec!["on"]),
    ] {
        let bound = &report.methods[method];
        assert_eq!(
            bound.fields.iter().map(String::as_str).collect::<Vec<_>>(),
            expected,
            "{method}"
        );
        assert!(bound.unresolved.is_empty(), "{method}: {bound:?}");
    }
}

#[test]
fn short_circuit_is_literal_only_and_optional_payload_false_does_not_mean_no_match() {
    let report = fixture(
        r#"
    fn install(options:Options){options.handler("fixture.read",|_,params|async move {
        if false && params.get(dynamic()).is_some() {}
        if true || params.get(other_dynamic()).is_some() {}
        let absent=None;
        if let Some(key)=absent { let _=params.get(key); }
        if let Some(_)=Some(false) { let _=params.get("required"); }
        if runtime_condition() && params.get(unknown_key()).is_some() {}
        Ok(())
    });}
    "#,
    );
    let bound = &report.methods["fixture.read"];
    assert!(bound.fields.contains("required"));
    assert_eq!(bound.unresolved.len(), 1, "{bound:?}");
    assert!(
        bound
            .unresolved
            .iter()
            .next()
            .unwrap()
            .contains("dynamic field")
    );
}

#[test]
fn unknown_optional_keys_and_none_fallbacks_remain_unresolved_instead_of_becoming_inert() {
    for expression in [
        "if runtime_condition(){Some(\"cwd\")}else{None}",
        "None.unwrap_or(Some(dynamic_key()))",
        "unknown_optional_key()",
    ] {
        let report = fixture(&format!(
            "fn install(options:Options){{options.handler(\"fixture.read\",|_,params|async move{{let selected={expression};if let Some(key)=selected{{let _=params.get(key);}}Ok(())}});}}"
        ));
        assert!(
            !report.methods["fixture.read"].unresolved.is_empty(),
            "{expression}: {:?}",
            report.methods["fixture.read"]
        );
    }
}
