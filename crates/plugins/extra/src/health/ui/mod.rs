mod manager;

use brass::dom::{Render, TagBuilder};
use factdb::{ClassMeta, Id, Timestamp};
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
use time::OffsetDateTime;

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

const FORMAT_DATE_TIME: &[time::format_description::FormatItem<'static>] =
    time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]");

fn weightlog_create(on_created: impl Fn(WeightLogEntry) + 'static) -> TagBuilder {
    #[derive(Clone)]
    struct Values {
        weight: String,
        date: String,
    }

    let on_created = std::rc::Rc::new(on_created);

    let fmt = time::macros::format_description!("");

    let form = Form::new(Values {
        weight: String::new(),
        date: time::OffsetDateTime::now_utc().format(&fmt).unwrap(),
    })
    .on_submit_async(move |values| {
        let on_created = on_created.clone();
        let values = values.clone();

        Box::pin(async move {
            let dt = OffsetDateTime::parse(&values.date, &FORMAT_DATE_TIME)?;
            let datetime = Timestamp::from(dt);

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
