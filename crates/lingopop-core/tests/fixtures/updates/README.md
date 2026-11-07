# Update signature fixtures

TEST material only (password `test`); it is never used for releases. The real key lives outside this repository.

Regenerate from the repository root:

```bash
export LP_TEST_PW=test
D=crates/lingopop-core/tests/fixtures/updates
cargo run -p lingopop-release-tools -- keygen --secret $D/test.key --public $D/test.pub --password-env LP_TEST_PW
cargo run -p lingopop-release-tools -- keygen --secret /tmp/other.key --public $D/other.pub --password-env LP_TEST_PW
cargo run -p lingopop-release-tools -- sign --key $D/test.key --password-env LP_TEST_PW $D/LingoPop-0.2.1-test.zip
```

`other.pub` is a second, unrelated key (its secret half is discarded). Check with:
`cargo run -p lingopop-release-tools -- verify --public-key $D/test.pub $D/LingoPop-0.2.1-test.zip`.
