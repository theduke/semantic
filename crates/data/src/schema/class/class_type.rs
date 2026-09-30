use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ClassType {
    /// Globally unique class identifier.
    pub id: String,
    pub name: String,
    /// Exactly one inherited base class, if any.
    pub inherits: Option<crate::schema::class::class_ref::ClassRef>,
    /// Additional extension classes (mixin style).
    pub extends: Vec<crate::schema::class::class_ref::ClassRef>,
    /// Reject attributes not declared by this class, its base class, or its extensions.
    #[facet(rename = "semantic:class:strict_schema")]
    #[facet(default)]
    pub strict_schema: bool,
    /// Whether the UI offers this class for entity creation; omitted means allowed.
    #[facet(rename = "semantic:ui:creatable_in_ui")]
    #[facet(default)]
    pub creatable_in_ui: Option<bool>,
    /// Whether instances appear in generic UI listings. Only false excludes them;
    /// omitted or null means included. Explicit queries and dedicated views are unaffected.
    #[facet(rename = "semantic:ui:include_in_listings")]
    #[facet(default, skip_serializing_if = Option::is_none)]
    pub include_in_ui_listings: Option<bool>,
    /// Declared attributes that make up this class.
    pub attributes: BTreeMap<String, crate::schema::class::class_attribute::ClassAttribute>,
    /// Class-level constraints, including multi-field constraints.
    pub constraints: Vec<crate::schema::class::class_constraint::ClassConstraint>,
    pub meta: crate::schema::core::meta::Meta,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::ClassType;

    #[test]
    fn listing_inclusion_round_trips_and_accepts_missing_or_null() {
        let mut class = crate::filestore::file_class();
        let legacy = facet_json::to_string(&class).unwrap();
        assert!(!legacy.contains("semantic:ui:include_in_listings"));
        for flag in [false, true] {
            class.include_in_ui_listings = Some(flag);
            let encoded = facet_json::to_string(&class).unwrap();
            assert!(encoded.contains(&format!("\"semantic:ui:include_in_listings\":{flag}")));
            assert_eq!(
                facet_json::from_str::<ClassType>(&encoded)
                    .unwrap()
                    .include_in_ui_listings,
                Some(flag)
            );
        }
        assert_eq!(
            facet_json::from_str::<ClassType>(&legacy)
                .unwrap()
                .include_in_ui_listings,
            None
        );
        let with_null = legacy.replacen('{', "{\"semantic:ui:include_in_listings\":null,", 1);
        assert_eq!(
            facet_json::from_str::<ClassType>(&with_null)
                .unwrap()
                .include_in_ui_listings,
            None
        );
    }

    #[test]
    fn creatable_in_ui_round_trips_and_is_optional() {
        let mut class = crate::filestore::file_class();
        for flag in [false, true] {
            class.creatable_in_ui = Some(flag);
            let encoded = facet_json::to_string(&class).unwrap();
            assert!(encoded.contains(&format!("\"semantic:ui:creatable_in_ui\":{flag}")));
            assert_eq!(
                facet_json::from_str::<ClassType>(&encoded)
                    .unwrap()
                    .creatable_in_ui,
                Some(flag)
            );
            let legacy = encoded.replace(&format!("\"semantic:ui:creatable_in_ui\":{flag},"), "");
            assert_eq!(
                facet_json::from_str::<ClassType>(&legacy)
                    .unwrap()
                    .creatable_in_ui,
                None
            );
        }
    }

    #[test]
    fn strict_schema_uses_namespaced_key_and_defaults_to_false_when_missing() {
        let class = ClassType {
            id: "example:article".to_string(),
            name: "Article".to_string(),
            inherits: None,
            extends: Vec::new(),
            strict_schema: true,
            creatable_in_ui: None,
            include_in_ui_listings: None,
            attributes: BTreeMap::new(),
            constraints: Vec::new(),
            meta: crate::schema::Meta::default(),
        };
        let encoded = facet_json::to_string(&class).expect("class should serialize");
        assert!(encoded.contains("\"semantic:class:strict_schema\":true"));
        let without_flag = encoded.replace("\"semantic:class:strict_schema\":true,", "");

        let decoded = facet_json::from_str::<ClassType>(&without_flag)
            .expect("legacy class without semantic:class:strict_schema should deserialize");

        assert!(!decoded.strict_schema);
    }
}
