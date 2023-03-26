mod alert;

pub use self::alert::Alert;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    Primary,
    Secondary,
    Success,
    Danger,
    Warning,
    Info,
    Light,
    Dark,
}
