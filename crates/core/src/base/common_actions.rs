use factordb::{
    prelude::{AttrMapExt, AttributeDescriptor, Db, Id, Patch, Timestamp},
    AnyError,
};

use super::{AttrLastVisitTime, AttrVisitCount};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RecordEntityVisit {
    pub entity_id: Id,
    pub time: Option<Timestamp>,
}

impl RecordEntityVisit {
    pub async fn run(self, db: &Db) -> Result<(), AnyError> {
        // TODO: use atomic counter update.
        let entity = db.entity(self.entity_id).await?;

        let old_count = entity.get_attr::<AttrVisitCount>().unwrap_or(0);

        let time = self.time.unwrap_or_else(|| Timestamp::now());

        let patch = Patch::new()
            .replace(AttrVisitCount::QUALIFIED_NAME, old_count + 1)
            .replace(AttrLastVisitTime::QUALIFIED_NAME, time.as_millis());

        db.patch(self.entity_id, patch).await?;

        tracing::debug!(entity_id=%self.entity_id, "recorded entity visit");

        Ok(())
    }
}
