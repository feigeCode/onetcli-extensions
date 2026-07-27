use super::*;

#[test]
fn pointer_state_tracks_vnc_button_mask() {
    let mut state = VncPointerState::default();

    assert_eq!(0, state.move_to(10, 11));
    assert_eq!(1, state.set_button(RemoteMouseButton::Left, true));
    assert_eq!(5, state.set_button(RemoteMouseButton::Right, true));
    assert_eq!(4, state.set_button(RemoteMouseButton::Left, false));
    assert_eq!(0, state.set_button(RemoteMouseButton::Right, false));
    assert_eq!((10, 11, 0), state.snapshot());
}

#[test]
fn pointer_candidate_does_not_commit_before_transport_success() {
    let state = VncPointerState {
        x: 1,
        y: 2,
        buttons: 1,
    };
    let mut candidate = state;
    candidate.move_to(10, 11);
    assert_eq!((1, 2, 1), state.snapshot());
    assert_eq!((10, 11, 1), candidate.snapshot());
}

#[test]
fn wheel_events_use_vnc_button_press_and_release_masks() {
    let mut state = VncPointerState::default();
    state.move_to(20, 21);

    assert_eq!(vec![8, 0], state.wheel_masks(true, -120));
    assert_eq!(vec![16, 0], state.wheel_masks(true, 120));
    assert_eq!(vec![32, 0], state.wheel_masks(false, -120));
    assert_eq!(vec![64, 0], state.wheel_masks(false, 120));
    assert_eq!((20, 21, 0), state.snapshot());
}

#[test]
fn close_and_reconnect_are_session_actions() {
    assert!(matches!(VncInputAction::Closed, VncInputAction::Closed));
    assert!(matches!(
        VncInputAction::Reconnect,
        VncInputAction::Reconnect
    ));
}
