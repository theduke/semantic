use super::Route;
use dioxus::prelude::*;
use futures::StreamExt;
use semantic_data::{Object, Value};
use semantic_ui_core::{use_active_scope_id, use_rpc_client};

#[derive(Clone, PartialEq)]
struct Candidate {
    plugin: String,
    export: String,
    source_export: String,
    generation: u64,
    title: String,
    priority: i32,
    status: String,
    reason: Option<String>,
}
fn candidates(value: Value) -> Result<Vec<Candidate>, String> {
    let Value::List(values) = value else {
        return Err("Invalid importer list".into());
    };
    values
        .into_iter()
        .map(|value| {
            let Value::Object(value) = value else {
                return Err("Invalid importer".into());
            };
            let text = |name: &str| {
                value
                    .get(name)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("Missing importer {name}"))
            };
            let descriptor = semantic_data::import::SourceDescriptor::from_value(
                value
                    .get("descriptor")
                    .cloned()
                    .ok_or("Missing importer descriptor")?,
            )
            .map_err(|e| e.to_string())?;
            Ok(Candidate {
                plugin: text("plugin_id")?,
                export: text("export")?,
                source_export: text("source_export")?,
                generation: match value.get("generation") {
                    Some(Value::U64(n)) => *n,
                    _ => return Err("Invalid importer generation".into()),
                },
                title: descriptor.title,
                priority: match value.get("priority") {
                    Some(Value::I32(n)) => *n,
                    _ => descriptor.default_priority,
                },
                status: text("status")?,
                reason: value
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn preview_fetcher<'a>(
    choice: &Candidate,
    available: &'a [Candidate],
) -> Result<&'a Candidate, String> {
    let fetcher = available
        .iter()
        .find(|candidate| {
            candidate.plugin == choice.plugin
                && candidate.source_export == choice.source_export
                && candidate.status == "supported"
        })
        .ok_or("Selected importer does not offer a supported fetch operation")?;
    if fetcher.generation != choice.generation {
        return Err("Plugin changed; find importers again".into());
    }
    Ok(fetcher)
}

#[component]
pub fn ImportPage() -> Element {
    let scope = use_active_scope_id();
    rsx! {ImportView {key:"{scope:?}",scope}}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(export: &str, source_export: &str) -> Candidate {
        Candidate {
            plugin: "multi-source".into(),
            export: export.into(),
            source_export: source_export.into(),
            generation: 1,
            title: export.into(),
            priority: 0,
            status: "supported".into(),
            reason: None,
        }
    }

    #[test]
    fn preview_follows_source_mapping_instead_of_export_order() {
        let choice = candidate("import-a", "source-b");
        let available = [
            candidate("fetch-a", "source-a"),
            candidate("fetch-b", "source-b"),
        ];
        assert_eq!(
            preview_fetcher(&choice, &available).unwrap().export,
            "fetch-b"
        );
        assert!(preview_fetcher(&choice, &available[..1]).is_err());
    }

    #[test]
    fn preview_requires_supported_source_from_same_plugin_generation() {
        let choice = candidate("import", "source");
        let mut fetcher = candidate("fetch", "source");
        fetcher.plugin = "another-plugin".into();
        assert!(preview_fetcher(&choice, &[fetcher.clone()]).is_err());
        fetcher.plugin = choice.plugin.clone();
        fetcher.status = "unsupported".into();
        assert!(preview_fetcher(&choice, &[fetcher.clone()]).is_err());
        fetcher.status = "supported".into();
        fetcher.generation += 1;
        assert_eq!(
            preview_fetcher(&choice, &[fetcher]).err().as_deref(),
            Some("Plugin changed; find importers again")
        );
    }
}
#[component]
fn ImportView(scope: Option<String>) -> Element {
    let rpc = use_rpc_client();
    let mut url = use_signal(String::new);
    let mut choices = use_signal(Vec::<Candidate>::new);
    let mut selected = use_signal(|| None::<usize>);
    let mut busy = use_signal(|| false);
    let mut searched = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut started = use_signal(|| None::<String>);
    let mut preview = use_signal(|| None::<String>);
    let preview_rpc = rpc.clone();
    let preview_scope = scope.clone();
    let fetch_preview = use_callback(move |_| {
        let rpc = preview_rpc.clone();
        let scope = preview_scope.clone();
        let requested_url = url();
        let Some(choice) = selected().and_then(|index| choices().get(index).cloned()) else {
            return;
        };
        busy.set(true);
        error.set(None);
        preview.set(None);
        spawn(async move {
            let result: Result<String, String> = async {
                let mut payload = Object::new();
                payload.insert("url", requested_url);
                if let Some(scope) = scope {
                    payload.insert("scope_id", scope);
                }
                payload.insert("operation", String::from("fetch"));
                let available = candidates(
                    rpc.invoke_value("semantic.import.candidates", Value::Object(payload.clone()))
                        .await
                        .map_err(|e| e.to_string())?,
                )?;
                let fetcher = preview_fetcher(&choice, &available)?;
                payload.remove("operation");
                payload.insert("plugin_id", fetcher.plugin.clone());
                payload.insert("export", fetcher.export.clone());
                payload.insert("generation", fetcher.generation);
                use semantic_rpc::interface::{
                    InvocationArgument, InvocationOutput, StreamEvent, ValidatedInvocation,
                };
                let output = rpc
                    .invoke_interface(ValidatedInvocation {
                        export: "application".into(),
                        method: "fetch_source".into(),
                        arguments: vec![InvocationArgument::Value(Value::Object(payload))],
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                let InvocationOutput::Stream(mut stream) = output else {
                    return Err("Expected fetched content".into());
                };
                while let Some(event) = stream.next().await {
                    match event.map_err(|e| e.to_string())? {
                        StreamEvent::Item(value) => {
                            match semantic_data::import::ContentEvent::from_value(value)
                                .map_err(|e| e.to_string())?
                            {
                                semantic_data::import::ContentEvent::FileStart(file) => {
                                    return Ok(format!(
                                        "{} · {}{}",
                                        file.filename.as_deref().unwrap_or("File"),
                                        file.mime_type,
                                        file.expected_size
                                            .map(|size| format!(" · {size} bytes"))
                                            .unwrap_or_default()
                                    ));
                                }
                                semantic_data::import::ContentEvent::Entity(entity) => {
                                    return Ok(format!(
                                        "{} · {} attributes",
                                        entity.class,
                                        entity.attributes.len()
                                    ));
                                }
                                _ => {}
                            }
                        }
                        StreamEvent::End(_) => return Ok("Source contains no items".into()),
                    }
                }
                Err("Fetched content ended before metadata".into())
            }
            .await;
            match result {
                Ok(metadata) => preview.set(Some(metadata)),
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    });
    let action = use_callback(move |start: bool| {
        let rpc = rpc.clone();
        let scope = scope.clone();
        let requested_url = url();
        let choice = selected().and_then(|index| choices().get(index).cloned());
        if start && choice.is_none() {
            return;
        }
        busy.set(true);
        error.set(None);
        started.set(None);
        preview.set(None);
        spawn(async move {
            let mut payload = Object::new();
            payload.insert("url", requested_url);
            if let Some(scope) = scope {
                payload.insert("scope_id", scope);
            }
            let command = if start {
                let choice = choice.expect("selected importer");
                payload.insert("plugin_id", choice.plugin);
                payload.insert("export", choice.export);
                payload.insert("generation", choice.generation);
                "semantic.import.start_source"
            } else {
                "semantic.import.candidates"
            };
            match rpc.invoke_value(command, Value::Object(payload)).await {
                Ok(value) if start => match value {
                    Value::Object(o) => match o.get("id").and_then(Value::as_str) {
                        Some(id) => started.set(Some(id.to_owned())),
                        None => error.set(Some("Missing job ID".into())),
                    },
                    _ => error.set(Some("Invalid import response".into())),
                },
                Ok(value) => match candidates(value) {
                    Ok(found) => {
                        selected.set(found.iter().position(|c| c.status == "supported"));
                        choices.set(found);
                        searched.set(true);
                    }
                    Err(message) => error.set(Some(message)),
                },
                Err(err) => error.set(Some(err.to_string())),
            }
            busy.set(false);
        });
    });
    rsx! {
        section {class:"semantic-page",
            h1 {"Import a URL"}
            p {"Choose an importer to save this URL as ordinary entities or files. Reimporting the same source replaces its previous data."}
            label {r#for:"import-url","URL"}
            input {id:"import-url",r#type:"url",placeholder:"https://example.com/document.pdf",value:url,disabled:busy(),oninput:move |event|{url.set(event.value());choices.set(Vec::new());selected.set(None);searched.set(false);started.set(None);preview.set(None);}}
            button {disabled:busy()||url().trim().is_empty(),onclick:move |_|action.call(false),"Find importers"}
            if let Some(message)=error() {p {role:"alert","{message}"}}
            if searched() {
                if choices().is_empty() {p {"No matching importers are available."}}
                else {
                    label {r#for:"import-choice","Importer"}
                    select {id:"import-choice",disabled:busy(),value:selected().map(|n|n.to_string()).unwrap_or_default(),onchange:move |event|selected.set(event.value().parse().ok()),
                        if selected().is_none() {option {value:"",disabled:true,"No supported importer"}}
                        for (index,choice) in choices().iter().enumerate() {
                            option {value:"{index}",disabled:choice.status!="supported","{choice.title} — priority {choice.priority} ({choice.plugin}/{choice.export})"}
                        }
                    }
                    p {"The highest-priority supported importer is selected by default. You can choose another explicitly."}
                    for choice in choices().iter().filter(|choice|choice.status!="supported") {
                        p {"{choice.title}: {choice.status} — {choice.reason.as_deref().unwrap_or(\"\")}"}
                    }
                    button {disabled:busy()||selected().is_none(),onclick:move |_|action.call(true),"Start import"}
                    button {disabled:busy()||selected().is_none(),onclick:move |_|fetch_preview.call(()),"Fetch preview"}
                }
            }
            if busy() {p {role:"status","Working…"}}
            if let Some(metadata)=preview() {p {"{metadata}"} p {"Preview reads source metadata without saving. Starting an import fetches the source again."}}
            if let Some(id)=started() {p {role:"status","Import started: {id}. " Link {to:Route::JobsPage,"View jobs"}}}
            p {Link {to:Route::JobsPage,"Jobs, progress and cancellation"}}
        }
    }
}
