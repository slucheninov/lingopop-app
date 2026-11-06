use std::path::PathBuf;

use lingopop_platform::fake::FakeLoginItem;
use lingopop_platform::{run_value, LoginItem};

#[test]
fn run_value_quotes_the_path() {
    let exe = PathBuf::from(r"C:\Program Files\LingoPop\lingopop.exe");
    assert_eq!(
        run_value(&exe),
        "\"C:\\Program Files\\LingoPop\\lingopop.exe\""
    );
}

#[test]
fn fake_login_item_records_calls() {
    let item = FakeLoginItem::new();
    let exe = PathBuf::from("lingopop.exe");
    item.set_enabled(true, &exe).expect("enable");
    item.set_enabled(false, &exe).expect("disable");
    assert_eq!(item.calls(), vec![(true, exe.clone()), (false, exe)]);
}

#[test]
fn fake_login_item_records_open_settings_and_can_fail_it() {
    let item = FakeLoginItem::new();
    item.open_settings().expect("open");
    assert_eq!(item.settings_opened(), 1);
    item.fail_next();
    assert!(item.open_settings().is_err());
    assert_eq!(item.settings_opened(), 2);
}
