mod pipe;
mod process;
mod supervisor;
pub(super) fn supervisor_entry() -> Option<i32> {
    supervisor::entry()
}
pub(super) fn prepare_supervisor() {
    supervisor::prepare();
}
pub(super) fn schedule_restart() -> Result<(), String> {
    supervisor::schedule()
}
pub(super) fn residency_pulse(probe: &crate::residency::Probe) -> Result<bool, String> {
    supervisor::pulse(probe)
}
pub(super) fn residency_available() -> bool {
    supervisor::residency_available()
}

#[cfg(test)]
mod tests;
