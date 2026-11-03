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
