#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LockState {
    Init,
    Input,
    Wait,
    Fail,
    Success,
}
