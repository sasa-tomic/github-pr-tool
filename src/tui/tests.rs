use super::*;

#[test]
fn test_tabs_state_new() {
    let tabs = TabsState::new(vec!["Tab1", "Tab2", "Tab3"]);
    assert_eq!(tabs.titles, vec!["Tab1", "Tab2", "Tab3"]);
    assert_eq!(tabs.index, 0);
}

#[test]
fn test_tabs_state_next() {
    let mut tabs = TabsState::new(vec!["Tab1", "Tab2", "Tab3"]);

    tabs.next();
    assert_eq!(tabs.index, 1);

    tabs.next();
    assert_eq!(tabs.index, 2);

    // Should wrap around to 0
    tabs.next();
    assert_eq!(tabs.index, 0);
}

#[test]
fn test_tabs_state_previous() {
    let mut tabs = TabsState::new(vec!["Tab1", "Tab2", "Tab3"]);

    // Should wrap around to last index
    tabs.previous();
    assert_eq!(tabs.index, 2);

    tabs.previous();
    assert_eq!(tabs.index, 1);

    tabs.previous();
    assert_eq!(tabs.index, 0);
}

#[test]
fn test_app_new() {
    let app = App::new("Test App");
    assert_eq!(app.title, "Test App");
    assert!(!app.should_quit);
    assert_eq!(app.tabs.index, 0);
    assert_eq!(app.tabs.titles, vec!["Logs", "Errors", "Details"]);
    assert!(app.logs.is_empty());
    assert!(app.errors.is_empty());
    assert_eq!(app.progress, 0.0);
    assert!(app.details.is_empty());
    assert!(!app.error_tab_blink);
    assert_eq!(app.blink_timer, 0);
}

#[test]
fn test_app_navigation() {
    let mut app = App::new("Test App");

    app.on_right();
    assert_eq!(app.tabs.index, 1);

    app.on_right();
    assert_eq!(app.tabs.index, 2);

    app.on_left();
    assert_eq!(app.tabs.index, 1);
}

#[test]
fn test_app_switch_to_tab() {
    let mut app = App::new("Test App");

    app.switch_to_tab(2);
    assert_eq!(app.tabs.index, 2);
}

#[test]
fn test_app_update_progress() {
    let mut app = App::new("Test App");

    app.update_progress(0.5);
    assert_eq!(app.progress, 0.5);

    app.update_progress(1.0);
    assert_eq!(app.progress, 1.0);
}

#[test]
fn test_app_add_log() {
    let mut app = App::new("Test App");

    app.add_log("INFO", "Test message");
    assert_eq!(app.logs.len(), 1);
    assert_eq!(app.logs[0].0, "INFO");
    assert_eq!(app.logs[0].1, "Test message");

    app.add_log("ERROR", String::from("Error message"));
    assert_eq!(app.logs.len(), 2);
    assert_eq!(app.logs[1].0, "ERROR");
    assert_eq!(app.logs[1].1, "Error message");
}

#[test]
fn test_app_add_error() {
    let mut app = App::new("Test App");

    app.add_error("Test error");
    assert_eq!(app.errors.len(), 1);
    assert_eq!(app.errors[0], "Test error");
    assert_eq!(app.logs.len(), 1);
    assert_eq!(app.logs[0].0, "ERROR");
    assert_eq!(app.logs[0].1, "Test error");
    assert!(app.error_tab_blink);
    assert_eq!(app.blink_timer, 10);
}

#[test]
fn test_app_add_error_multiline() {
    let mut app = App::new("Test App");

    app.add_error("Line 1\nLine 2\nLine 3");
    assert_eq!(app.errors.len(), 3);
    assert_eq!(app.errors[0], "Line 1");
    assert_eq!(app.errors[1], "Line 2");
    assert_eq!(app.errors[2], "Line 3");
    assert_eq!(app.logs.len(), 3);
}

#[test]
fn test_app_update_details() {
    let mut app = App::new("Test App");

    app.update_details("Test details".to_string());
    assert_eq!(app.details, "Test details");
}

#[test]
fn test_app_start_error_blink() {
    let mut app = App::new("Test App");

    app.start_error_blink();
    assert!(app.error_tab_blink);
    assert_eq!(app.blink_timer, 10);
}

#[test]
fn test_app_update_blink() {
    let mut app = App::new("Test App");

    app.start_error_blink();
    assert_eq!(app.blink_timer, 10);

    app.update_blink();
    assert_eq!(app.blink_timer, 9);
    assert!(app.error_tab_blink);

    // Fast forward to end
    for _ in 0..9 {
        app.update_blink();
    }
    assert_eq!(app.blink_timer, 0);
    assert!(!app.error_tab_blink);
}

#[test]
fn test_app_update_blink_no_effect_when_not_blinking() {
    let mut app = App::new("Test App");

    app.update_blink();
    assert_eq!(app.blink_timer, 0);
    assert!(!app.error_tab_blink);
}
