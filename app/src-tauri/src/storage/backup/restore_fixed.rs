//! Typed fixed-data reconstruction. Results stay in private restore staging;
//! this module does not publish meetings or make missing engines available.
#[cfg(test)]
use super::restore_ids::RestoreIds;
use super::*;
use crate::audio_import::ImportOptions;
use crate::storage::vocabulary::VocabularySnapshot;
use serde_json::Value;

pub(in crate::storage::backup) mod workspace;

pub(super) struct Definition {
    pub domain: &'static str,
    pub meeting: String,
    pub old: String,
    pub lineage: Option<&'static str>,
}

pub(super) fn visit_definitions(
    source: &Connection,
    cancelled: &impl Fn() -> bool,
    visit: &mut impl FnMut(Definition) -> Result<(), String>,
) -> Result<(), String> {
    for sql in [
        "SELECT meeting_id,payload_json,'vocabulary' FROM vocabulary_snapshots",
        "SELECT meeting_id,parameters_json,'parameters' FROM processing_jobs",
        "SELECT meeting_id,config_json,'config' FROM analysis_revisions",
        "SELECT i.meeting_id,o.options_json,'options' FROM audio_import_options o JOIN audio_imports i ON i.id=o.import_id",
    ] {
        let mut query=source.prepare(sql).map_err(|e|e.to_string())?;
        let mut rows=query.query([]).map_err(|e|e.to_string())?;
        while let Some(row)=rows.next().map_err(|e|e.to_string())? {
            check_cancelled(cancelled)?;
            let meeting:String=row.get(0).map_err(|e|e.to_string())?;
            let text:String=row.get(1).map_err(|e|e.to_string())?;
            let kind:String=row.get(2).map_err(|e|e.to_string())?;
            let value:Value=serde_json::from_str(&text).map_err(|e|e.to_string())?;
            let vocabulary=if kind=="vocabulary" {Some(&value)} else {value.get("vocabulary").filter(|v|!v.is_null())};
            if let Some(value)=vocabulary {
                let snapshot:VocabularySnapshot=serde_json::from_value(value.clone()).map_err(|e|e.to_string())?;
                snapshot.validate()?;
                if snapshot.meeting_id!=meeting {return Err("固定用語の会議が一致しません".into());}
                visit(Definition{domain:"vocabulary_snapshots",meeting:meeting.clone(),old:snapshot.id,lineage:None})?;
                for entry in snapshot.entries {
                    visit(Definition{domain:"vocabulary_entries",meeting:if entry.scope=="global" {String::new()} else {meeting.clone()},old:entry.entry.id,lineage:None})?;
                }
            }
            if kind=="options" {
                let options:ImportOptions=serde_json::from_value(value.clone()).map_err(|e|e.to_string())?;
                options.validate()?;
                for entry in options.meeting_vocabulary {visit(Definition{domain:"vocabulary_entries",meeting:meeting.clone(),old:entry.id,lineage:None})?;}
            }
            if kind=="config" {
                for key in ["sourceRecognitionLineages", "manualAssignmentLineages"] {
                    if let Some(lineages) = value.get(key) {
                        let lineages: Vec<String> = serde_json::from_value(lineages.clone()).map_err(|e| e.to_string())?;
                        for old in lineages {
                            visit(Definition { domain: "utterance_lineage", meeting: meeting.clone(), old, lineage: None })?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) struct FixedData<'p, 's> {
    ids: &'p RestoreIds<'s>,
}
#[cfg(test)]
impl<'p, 's> FixedData<'p, 's> {
    #[cfg(test)]
    pub(super) fn new(
        ids: &'p RestoreIds<'s>,
        _parent: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        ids.source().verify_validation_database(cancelled)?;
        Ok(Self { ids })
    }
}
