use sun_observations::certifications::*;

use super::*;
use crate::correction::CorrectionFeatures;

#[derive(Clone, DomainInput)]
enum Permission {
    Disabled,
    Enabled,
}

impl Permission {
    fn enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }
}

#[derive(Clone, DomainInput)]
struct PolicyInput {
    autocorrect: Permission,
    auto_layout: Permission,
    autocomplete: Permission,
    suppressed: Permission,
    known: Permission,
}

#[test]
fn effective_policy_preserves_independent_global_permissions_and_full_input_suppression() {
    let mut runner =
        certification("effective application correction policy").executor(|input: &PolicyInput| {
            let application = ApplicationIdentity::try_new(r"C:\Apps\example.exe").unwrap();
            let overrides =
                ApplicationOverrides::from_applications(if input.suppressed.enabled() {
                    vec![application.clone()]
                } else {
                    vec![]
                });
            let settings = AppSettings::default()
                .with_enable_autocorrections(input.autocorrect.enabled())
                .with_enable_auto_keyboard_switches(input.auto_layout.enabled())
                .with_enable_autocomplete(input.autocomplete.enabled());
            let foreground = input.known.enabled().then_some(&application);
            Ok::<_, ()>((
                overrides.effective_policy(settings, foreground),
                settings,
                overrides.suppresses_input(foreground),
            ))
        });
    runner.domain(domain!(PolicyInput {
        autocorrect.all_enum_values(),
        auto_layout.all_enum_values(),
        autocomplete.all_enum_values(),
        suppressed.all_enum_values(),
        known.all_enum_values(),
    }));
    runner.cases_budget(64).require_pairwise_coverage();
    let case = runner.case();
    runner.check(
        fact!(case.result().0.allows(CorrectionFeatures::new(true, false))).expect(fact!(
            case.input().known.enabled()
                && !case.input().suppressed.enabled()
                && case.input().autocorrect.enabled()
        )),
    );
    runner.check(
        fact!(case.result().0.allows(CorrectionFeatures::new(false, true))).expect(fact!(
            case.input().known.enabled()
                && !case.input().suppressed.enabled()
                && case.input().auto_layout.enabled()
        )),
    );
    runner.check(
        fact!(case.result().0.allows(CorrectionFeatures::new(true, true))).expect(fact!(
            case.input().known.enabled()
                && !case.input().suppressed.enabled()
                && case.input().autocorrect.enabled()
                && case.input().auto_layout.enabled()
        )),
    );
    runner.check(
        fact!(case.result().1.enable_autocomplete())
            .expect(fact!(case.input().autocomplete.enabled())),
    );
    runner.check(
        fact!(case.result().1.enable_autocorrections())
            .expect(fact!(case.input().autocorrect.enabled())),
    );
    runner.check(
        fact!(case.result().1.enable_auto_keyboard_switches())
            .expect(fact!(case.input().auto_layout.enabled())),
    );
    runner.check(fact!(case.result().2).expect(fact!(
        case.input().known.enabled() && case.input().suppressed.enabled()
    )));
    runner.assert_certify();
}

#[test]
fn full_executable_identities_normalize_alias_spelling_without_basename_collisions() {
    let mut c = certification("canonical full executable identity");
    let identity = ApplicationIdentity::try_new("C:/Apps/Editor.EXE").unwrap();
    c.check_fact_expect(
        "extended drive path",
        identity.clone(),
        ApplicationIdentity::try_new(r"\\?\C:\Apps\Editor.exe").unwrap(),
    );
    c.check_fact_expect(
        "different drive is a different application",
        identity,
        NotEqual(ApplicationIdentity::try_new(r"D:\Apps\Editor.exe").unwrap()),
    );
    c.check_fact_expect(
        "extended UNC path",
        ApplicationIdentity::try_new(r"\\?\UNC\Server\Share\Editor.exe").unwrap(),
        ApplicationIdentity::try_new(r"\\server\share\editor.exe").unwrap(),
    );
    c.check_fact_expect(
        "executable extension is not restricted to EXE",
        ApplicationIdentity::try_new(r"C:\Apps\Editor.COM")
            .unwrap()
            .file_name(),
        "editor.com",
    );
    c.assert_certify();
}

#[test]
fn ambiguous_and_relative_executable_identities_are_rejected() {
    let mut runner = certification("invalid application identities fail closed")
        .executor(|path: &&str| Ok::<_, ()>(ApplicationIdentity::try_new(path)));
    runner.data_source([
        "editor.exe",
        r"C:editor.exe",
        r"C:\Apps\..\editor.exe",
        r"C:\Apps\\editor.exe",
        r"C:\Apps \editor.exe",
        r"C:\Apps\editor.exe:stream",
        r"\\.\editor.exe",
        "C:\\bad\0.exe",
    ]);
    let case = runner.case();
    runner.check(fact!(case.result().is_err()).expect(true));
    runner.assert_certify();
}
