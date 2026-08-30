/// Data state shared by asynchronous route surfaces.
///
/// Keep route-specific copy and actions in the presentation components. The
/// payload should normally be a cheap shared value such as `Rc<T>` rather than a
/// large collection cloned for rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncState<T, E = String> {
    Loading,
    Ready(T),
    Refreshing(T),
    Empty,
    Error(E),
}

impl<T, E> AsyncState<T, E> {
    /// Returns retained or ready data without cloning it.
    pub fn data(&self) -> Option<&T> {
        match self {
            Self::Ready(data) | Self::Refreshing(data) => Some(data),
            Self::Loading | Self::Empty | Self::Error(_) => None,
        }
    }

    /// Returns the current error, if the state is terminally failed.
    pub fn error(&self) -> Option<&E> {
        match self {
            Self::Error(error) => Some(error),
            Self::Loading | Self::Ready(_) | Self::Refreshing(_) | Self::Empty => None,
        }
    }

    /// Whether the surface is performing an initial load or retained refresh.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Loading | Self::Refreshing(_))
    }

    /// Transforms the data payload while preserving the state and error.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> AsyncState<U, E> {
        match self {
            Self::Loading => AsyncState::Loading,
            Self::Ready(data) => AsyncState::Ready(map(data)),
            Self::Refreshing(data) => AsyncState::Refreshing(map(data)),
            Self::Empty => AsyncState::Empty,
            Self::Error(error) => AsyncState::Error(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AsyncState;

    #[test]
    fn retained_data_is_available_while_refreshing() {
        let state: AsyncState<_, String> = AsyncState::Refreshing(vec![1, 2]);

        assert_eq!(state.data(), Some(&vec![1, 2]));
        assert!(state.is_busy());
        assert_eq!(state.error(), None);
    }

    #[test]
    fn map_preserves_the_state_variant() {
        let state: AsyncState<_, String> = AsyncState::Ready("entity");

        assert_eq!(state.map(str::len), AsyncState::Ready(6));
    }
}
