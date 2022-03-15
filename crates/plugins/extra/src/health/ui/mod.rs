mod manager;

use brass::dom::{Render, TagBuilder};
use chrono::TimeZone;
use factordb::{
    prelude::{EntityDescriptor, Id, Timestamp},
    AnyError,
};
use semantic_core::plugin::PluginDescriptor;
use semantic_ui_core::{
    components::{
        form::Form,
        util::{box_, form_field_input, subtitle_4, FormRenderer},
    },
    plugin::PluginMainRoute,
    routing::{PluginRoute, PluginRouter},
    validate::{StringDateTime, StringFloat},
    BrowserPlugin,
};

use super::WeightLogEntry;

impl BrowserPlugin for super::HealthPlugin {
    fn spec(&self) -> semantic_ui_core::BrowserPluginSpec {
        semantic_ui_core::BrowserPluginSpec {
            name: Self::NAME.to_string(),
            main_route: Some(PluginMainRoute {
                name: "Health".to_string(),
                route: HealthRouter::route_health(),
            }),
        }
    }

    fn register(&self, registry: &mut semantic_ui_core::Registry) {
        registry.ignore_entity_type(WeightLogEntry::QUALIFIED_NAME.to_string());
    }

    fn router(&self) -> Option<semantic_ui_core::routing::DynPluginRouter> {
        Some(Box::new(HealthRouter))
    }
}

struct HealthRouter;

impl HealthRouter {
    fn route_health() -> PluginRoute {
        PluginRoute {
            path: "/health/weight".to_string(),
            title: "Weight".to_string(),
            render: std::rc::Rc::new(|| {
                TagBuilder::from_node(manager::WeightlogManager {}.render().into_node().unwrap())
            }),
        }
    }
}

impl PluginRouter for HealthRouter {
    fn parse_path(&self, path: &[&str]) -> Option<semantic_ui_core::routing::PluginRoute> {
        match path {
            ["health", "weight"] => Some(Self::route_health()),
            _ => None,
        }
    }
}

fn weightlog_create(on_created: impl Fn(WeightLogEntry) + 'static) -> TagBuilder {
    #[derive(Clone)]
    struct Values {
        weight: String,
        date: String,
    }

    let on_created = std::rc::Rc::new(on_created);

    let form = Form::new(Values {
        weight: String::new(),
        date: chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string(),
    })
    .on_submit_async(move |values| {
        let on_created = on_created.clone();
        let values = values.clone();

        Box::pin(async move {
            let naive = chrono::NaiveDateTime::parse_from_str(&values.date, "%Y-%m-%d %H:%M")?;
            let local = chrono::offset::Local
                .from_local_datetime(&naive)
                .earliest()
                .ok_or_else(|| AnyError::msg("Could not determine local timezone"))?;
            let datetime = Timestamp::from_millis(local.timestamp_millis() as u64);

            let entry = WeightLogEntry {
                id: Id::random(),
                weight: values.weight.parse::<f64>()?,
                comment: None,
                datetime,
            };

            semantic_ui_core::context::api()
                .entity_create(entry.clone())
                .await?;
            on_created(entry);

            Ok(())
        })
    })
    .build();

    let form = FormRenderer::new(form.clone())
        .field(
            form.field_validated(|v| &mut v.weight, StringFloat),
            |field| form_field_input("Weight", field),
        )
        .field(
            form.field_validated(|v| &mut v.date, StringDateTime),
            |field| form_field_input("Date", field),
        )
        .buttons_submit("Create");

    box_().and(subtitle_4().and("Log Weight")).and(form)
}
