use std::fs;

use cockpit_recording::PayloadStore;

#[test]
fn content_addressed_payloads_deduplicate_and_verify_hashes() {
    let root = std::env::temp_dir().join(format!("cockpit-payload-test-{}", uuid::Uuid::new_v4()));
    let store = PayloadStore::new(&root).expect("payload store creates");
    let payload = br#"{"secret":"not a secret-bearing payload in this fixture"}"#;
    let first = store.put(payload).expect("payload stores");
    let second = store.put(payload).expect("duplicate payload stores");
    assert_eq!(first, second);
    assert_eq!(store.get(&first).expect("payload reads"), payload);
    assert!(store.path_for(&first).exists());
    let files = fs::read_dir(root.join(&first[7..9]))
        .expect("fanout directory reads")
        .count();
    assert_eq!(files, 1);
}
