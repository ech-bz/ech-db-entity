use ech_db_entity::{entity, entity_impl, Result};

#[derive(serde::Serialize, serde::Deserialize)]
pub enum ScratchEvent {
    Genesis { a: u64 },
}

#[entity(event = ScratchEvent, key = ())]
pub enum ScratchEntity {
    V1 { a: u64 },
}

#[entity_impl]
impl ScratchEntity {
    fn genesis(event: &ScratchEvent) -> Result<Self> {
        match event {
            ScratchEvent::Genesis { a } => Ok(Self::V1 { a: *a }),
        }
    }

    fn apply(&mut self, _event: &ScratchEvent) -> Result<()> {
        Ok(())
    }
}
