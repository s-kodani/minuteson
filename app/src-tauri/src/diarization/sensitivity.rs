//! Legacy serialized zero is supported; sensitivity adjustment is not enabled.
pub(crate) fn is_standard(value: &i32) -> bool {
    *value == 0
}
pub(crate) fn valid(value: i32) -> bool {
    value == 0
}
