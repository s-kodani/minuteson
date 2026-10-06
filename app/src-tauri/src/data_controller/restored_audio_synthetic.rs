//! Test-only access to the real controller and shared lease; no product API.
use super::*;
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;

pub(crate) struct Synthetic {
    controller: DataController,
    analysis: AnalysisController,
}
impl Synthetic {
    pub(crate) fn new(storage: &Storage) -> Self {
        let analysis = AnalysisController::default();
        if storage.deletion_resume_restart_required() {
            analysis.require_storage_restart();
        }
        Self {
            controller: DataController::default(),
            analysis,
        }
    }
    pub(crate) fn pending(&self, storage: &Storage, operation: &str) -> Result<Snapshot, String> {
        let lease = self.analysis.begin_audio_deletion_maintenance(
            storage,
            operation,
            false,
            false,
            &|| false,
        )?;
        let (s, _, previous) = self.controller.begin(Kind::AudioDeletion)?;
        Ok(self.finish(
            storage,
            Work::AudioDeletion(AudioTarget::Pending(operation.into()), previous),
            &s,
            &lease,
            &|| false,
            &|_| {},
        ))
    }
    pub(crate) fn fresh(&self, storage: &Storage, meeting: &str) -> Snapshot {
        let lease = self
            .analysis
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let (s, _, previous) = self.controller.begin(Kind::AudioDeletion).unwrap();
        self.finish(
            storage,
            Work::AudioDeletion(AudioTarget::Fresh(meeting.into()), previous),
            &s,
            &lease,
            &|| false,
            &|_| {},
        )
    }
    fn finish(
        &self,
        storage: &Storage,
        work: Work,
        s: &Snapshot,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Snapshot {
        self.controller
            .finish(
                s.token.as_ref().unwrap(),
                work.run_admitted(storage, check, progress, Some(lease)),
            )
            .unwrap()
    }
    pub(crate) fn request(s: &Snapshot, action: Action) -> Request {
        Request {
            token: s.token.clone().unwrap(),
            confirmation_id: s
                .audio_deletion_preview
                .as_ref()
                .unwrap()
                .confirmation_id
                .clone(),
            action,
        }
    }
    pub(crate) fn denied(&self, s: &Snapshot, action: Action) -> bool {
        self.controller
            .begin_request(Self::request(s, action))
            .is_err()
    }
    pub(crate) fn replay(&self, request: Request) -> bool {
        self.controller.begin_request(request).unwrap().is_none()
    }
    pub(crate) fn admission(&self) -> String {
        let state = self.controller.state.lock().unwrap();
        let Some(Selection::AudioDeletion(p)) = &state.selection else {
            panic!("no audio selection");
        };
        p.admission_operation().unwrap().to_owned()
    }
    pub(crate) fn run(&self, storage: &Storage, s: &Snapshot, action: Action) -> Snapshot {
        self.stop_at(storage, s, action, "NEVER")
    }
    pub(crate) fn stop_at(
        &self,
        storage: &Storage,
        s: &Snapshot,
        action: Action,
        phase: &str,
    ) -> Snapshot {
        let cancelled = AtomicBool::new(false);
        self.run_observed(
            storage,
            s,
            action,
            &|| cancelled.load(Ordering::Acquire),
            &|p| {
                if p == phase {
                    cancelled.store(true, Ordering::Release);
                }
            },
        )
    }
    pub(crate) fn run_observed(
        &self,
        storage: &Storage,
        s: &Snapshot,
        action: Action,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Snapshot {
        let lease = self
            .analysis
            .begin_audio_deletion_maintenance(storage, &self.admission(), false, false, &|| false)
            .unwrap();
        let (work, _, started) = self
            .controller
            .begin_request(Self::request(s, action))
            .unwrap()
            .unwrap();
        self.finish(storage, work, &started, &lease, check, progress)
    }
    pub(crate) fn page(&self, s: &Snapshot, offset: usize) -> audio_deletion::PathPage {
        self.controller
            .audio_deletion_page(
                &s.audio_deletion_preview.as_ref().unwrap().confirmation_id,
                offset,
            )
            .unwrap()
    }
}
