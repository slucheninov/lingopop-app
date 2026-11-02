use lingopop_platform::{fake::FakeSystem, SystemInfo};
#[test]
fn fake_system_records_opened_urls() {
    let s = FakeSystem::new(vec!["uk-UA".into()]);
    assert_eq!(s.ui_languages(), vec!["uk-UA"]);
    s.open_url("https://example.com").unwrap();
    assert_eq!(s.opened(), vec!["https://example.com"]);
    s.set_process_exits(false);
    assert!(!s.wait_for_process_exit(1, std::time::Duration::ZERO));
}
