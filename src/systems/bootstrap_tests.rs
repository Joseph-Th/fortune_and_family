use super::*;
use crate::test_support::rivergate_registry_for_test as test_registry;

mod names {
    use super::*;

    #[test]
    fn reject_empty_dynasty() {
        let registry = test_registry();
        let empty_dynasty = NewGameConfig {
            dynasty_name: "   ".to_owned(),
            ..NewGameConfig::default()
        };

        assert_eq!(
            build_new_game(registry, empty_dynasty),
            Err(NewGameError::EmptyDynastyName)
        );
    }

    #[test]
    fn reject_empty_founder() {
        let registry = test_registry();
        let empty_founder = NewGameConfig {
            founder_name: "\n\t".to_owned(),
            ..NewGameConfig::default()
        };

        assert_eq!(
            build_new_game(registry, empty_founder),
            Err(NewGameError::EmptyFounderName)
        );
    }

    #[test]
    fn normalize_internal_whitespace_at_the_input_boundary() {
        let registry = test_registry();
        let config = NewGameConfig {
            dynasty_name: "  House\tValeri  ".to_owned(),
            founder_name: "  Elian\n  Valeri  ".to_owned(),
            ..NewGameConfig::default()
        };

        let state = build_new_game(registry, config).expect("game must build");
        let dynasty = state
            .get_dynasty(state.player_dynasty_id())
            .expect("player dynasty must exist");
        let founder = state
            .characters()
            .get(dynasty.head_id())
            .expect("founder must exist");

        assert_eq!(dynasty.name(), "House Valeri");
        assert_eq!(founder.name(), "Elian Valeri");
    }

    #[test]
    fn reject_terminal_control_characters() {
        let registry = test_registry();

        assert_eq!(
            build_new_game(
                registry,
                NewGameConfig {
                    dynasty_name: "Valeri\u{1b}[31m".to_owned(),
                    ..NewGameConfig::default()
                }
            ),
            Err(NewGameError::InvalidDynastyNameCharacter {
                character: '\u{1b}',
            })
        );
    }

    #[test]
    fn reject_overlong_dynasty_by_character_count() {
        let registry = test_registry();
        let dynasty_name = "V".repeat(MAX_DYNASTY_NAME_CHARACTERS + 1);

        assert_eq!(
            build_new_game(
                registry,
                NewGameConfig {
                    dynasty_name,
                    ..NewGameConfig::default()
                }
            ),
            Err(NewGameError::DynastyNameTooLong {
                actual: MAX_DYNASTY_NAME_CHARACTERS + 1,
                maximum: MAX_DYNASTY_NAME_CHARACTERS,
            })
        );
    }

    #[test]
    fn reject_overlong_unicode_founder_by_character_count() {
        let registry = test_registry();
        let founder_name = "É".repeat(MAX_FOUNDER_NAME_CHARACTERS + 1);

        assert_eq!(
            build_new_game(
                registry,
                NewGameConfig {
                    founder_name,
                    ..NewGameConfig::default()
                }
            ),
            Err(NewGameError::FounderNameTooLong {
                actual: MAX_FOUNDER_NAME_CHARACTERS + 1,
                maximum: MAX_FOUNDER_NAME_CHARACTERS,
            })
        );
    }
}
