use rusty_whisper::server::{SlotGuard, Slots};

#[test]
fn slot_guard_name_remains_public_and_releases_capacity() {
    let slots = Slots::new(1);

    let guard: SlotGuard<'_> = slots.try_acquire().expect("slot should be available");
    assert!(slots.try_acquire().is_none());

    drop(guard);

    let reacquired: SlotGuard<'_> = slots
        .try_acquire()
        .expect("dropping SlotGuard should release capacity");
    drop(reacquired);
}
