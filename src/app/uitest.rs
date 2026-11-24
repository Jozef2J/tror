use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn sandbox(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("tror-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", home.join("data"));
        std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        std::env::set_var("LANG", "ru_RU.UTF-8");
    }
    home
}

fn app<'a>() -> Harness<'a, TrorApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| TrorApp::new(cc));
    harness.run_steps(4);
    harness
}

#[test]
#[ignore]
fn ui_starts_and_shows_menu() {
    sandbox("start");
    let harness = app();
    harness.get_by_label("Файл");
    harness.get_by_label("Правка");
}

#[allow(dead_code)]
fn labels(harness: &Harness<'_, TrorApp>) -> Vec<String> {
    harness
        .query_all(egui_kittest::kittest::by())
        .filter_map(|n| {
            use egui_kittest::kittest::NodeT;
            n.accesskit_node().label()
        })
        .collect()
}

fn wait(
    harness: &mut Harness<'_, TrorApp>,
    what: &str,
    done: impl Fn(&Harness<'_, TrorApp>) -> bool,
) {
    for _ in 0..300 {
        harness.step();
        if done(harness) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("не дождались: {what}");
}

#[test]
#[ignore]
fn ui_edit_and_save_file() {
    let home = sandbox("save");
    let file = home.join("index.php");
    std::fs::write(&file, "<?php\n").unwrap();
    let mut harness = app();
    harness.state_mut().open_path(&file, 0.0);
    harness.run_steps(3);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::End);
    harness.event(egui::Event::Text("echo 1;".into()));
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    harness.run_steps(3);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "<?php\necho 1;");
    let names: Vec<_> = std::fs::read_dir(&home)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(
        names
            .iter()
            .all(|n| !n.to_string_lossy().contains("trorpad-save")),
        "{names:?}"
    );
}

#[test]
#[ignore]
fn ui_database_search_finds_rows() {
    let home = sandbox("db");
    let path = home.join("shop.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE goods (id INTEGER PRIMARY KEY, title TEXT);
             INSERT INTO goods (title) VALUES ('Чайник Bosch'), ('Кружка');
             CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);
             INSERT INTO users (name) VALUES ('bosch fan');",
        )
        .unwrap();
    }
    let mut harness = app();
    {
        let state = harness.state_mut();
        state.remotes[0].store.databases = vec![crate::db::DbProfile {
            name: "shop".into(),
            kind: crate::db::DbKind::Sqlite,
            file: path.to_string_lossy().into_owned(),
            ..Default::default()
        }];
        let floats = state.settings.floats.clone();
        dock::show(&mut state.dock, Pane::Db, &floats);
    }
    harness.run_steps(3);
    let connect = harness
        .get_all_by_label("Подключить")
        .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .unwrap();
    connect.click();
    wait(&mut harness, "подключение к базе", |h| {
        h.query_by_label("Отключить").is_some()
    });
    harness
        .get_all_by_label("Поиск")
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap()
        .click();
    harness.run_steps(2);
    let row = harness
        .get_all_by_label("Найти")
        .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .unwrap()
        .rect()
        .center()
        .y;
    let field = harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .min_by(|a, b| {
            (a.rect().center().y - row)
                .abs()
                .total_cmp(&(b.rect().center().y - row).abs())
        })
        .unwrap();
    field.click();
    harness.run_steps(1);
    harness.event(egui::Event::Text("BOSCH".into()));
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
    wait(&mut harness, "результаты поиска", |h| {
        h.query_by_label_contains("Найдено строк").is_some()
    });
    harness.get_by_label_contains("Найдено строк: 2, таблиц: 2 из 2");
    harness.get_by_label("goods · строк: 1");
    harness.get_by_label("users · строк: 1");
}

#[test]
#[ignore]
fn ui_database_profiles_add_and_save() {
    sandbox("dbprof");
    let mut harness = app();
    {
        let state = harness.state_mut();
        let floats = state.settings.floats.clone();
        dock::show(&mut state.dock, Pane::Db, &floats);
    }
    harness.run_steps(3);
    let before = harness.state().remotes[0].store.databases.len();
    harness.get_by_label("Создать подключение").click();
    harness.run_steps(3);
    harness.get_by_label("Профили баз данных");
    harness
        .get_all_by_label("Добавить")
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap()
        .click();
    harness.run_steps(2);
    harness.get_by_label("Сохранить").click();
    harness.run_steps(3);
    assert!(harness.query_by_label("Профили баз данных").is_none());
    assert_eq!(harness.state().remotes[0].store.databases.len(), before + 2);
}

#[test]
#[ignore]
fn ui_remote_hosts_explains_empty_state() {
    sandbox("hosts");
    let mut harness = app();
    {
        let state = harness.state_mut();
        let floats = state.settings.floats.clone();
        dock::show(&mut state.dock, Pane::Hosts, &floats);
    }
    harness.run_steps(3);
    harness.get_by_label("Создать профиль");
    harness.state_mut().remotes[0]
        .store
        .profiles
        .push(crate::remote::Profile::new("site".into()));
    harness.run_steps(3);
    harness.get_by_label_contains("Здесь появятся файлы сервера");
}

#[test]
#[ignore]
fn ui_settings_open_and_switch_page() {
    sandbox("settings");
    let mut harness = app();
    {
        let state = harness.state_mut();
        let floats = state.settings.floats.clone();
        dock::show(&mut state.dock, Pane::Settings, &floats);
    }
    harness.run_steps(3);
    harness.get_by_label("Общие").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Установлена версия");
}

#[test]
#[ignore]
fn ui_find_and_replace_in_document() {
    let home = sandbox("find");
    let file = home.join("a.txt");
    std::fs::write(&file, "price price cost").unwrap();
    let mut harness = app();
    harness.state_mut().open_path(&file, 0.0);
    harness.run_steps(3);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::H);
    harness.run_steps(3);
    let mut inputs: Vec<_> = harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect().top())
        .collect();
    inputs.sort_by(|a, b| a.total_cmp(b));
    assert!(inputs.len() >= 2, "{inputs:?}");
    let field = |harness: &Harness<'_, TrorApp>, top: f32| {
        harness
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .find(|n| n.rect().top() == top)
            .unwrap()
            .click();
    };
    field(&harness, inputs[0]);
    harness.run_steps(1);
    harness.event(egui::Event::Text("price".into()));
    field(&harness, inputs[1]);
    harness.run_steps(1);
    harness.event(egui::Event::Text("cost2".into()));
    harness.run_steps(2);
    harness.get_by_label("Заменить все").click();
    harness.run_steps(3);
    let state = harness.state();
    assert_eq!(
        state.docs[state.active].rope.to_string(),
        "cost2 cost2 cost"
    );
}
