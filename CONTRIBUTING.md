# Contributing to Tror

**English** · [Русский](#русский)

Thank you for helping. Bug reports, ideas and pull requests are all welcome.

## Reporting a bug

Open an [issue](https://github.com/Jozef2J/tror/issues/new/choose) and choose "Bug report". The most useful reports include:

- the TrorPad version (shown on the start screen and in Settings → General) and the operating system;
- the steps that lead to the problem and what you expected instead;
- the lines from the "Log" panel around the moment of the error, if the problem involves a server, a database or the console. Remove passwords, host names and anything else private before posting.

Security problems are different: please do not open a public issue. See [SECURITY.md](SECURITY.md).

## Suggesting a feature

Open an issue and choose "Feature request". Describe the task you are trying to do, not only the button you would like to see — that helps find the simplest solution.

## Pull requests

1. Fork the repository and create a branch from `main`.
2. Build and test:
   ```
   cargo test
   cargo clippy
   ```
   Rust 1.95+ and a C compiler are required; on Linux and macOS also `perl` and `make`.

   Code structure is checked by [deslop-please](https://github.com/deterministic-machines/deslop-please): `cargo install deslop-please`, then `deslop-please --allow .deslop-please.allow`. New module cycles, field leaks and shallow modules fail CI. Fix the cause instead of hiding it; `.deslop-please.allow` lists existing debt and may only shrink.
3. Keep the change focused on one thing. Follow the style of the surrounding code: small direct functions, no comments in the code, the shared widgets from `src/widgets.rs` for any interface.
4. Every interface string is written in Russian inside `tr!("…")` and gets its English translation in `src/i18n/en.rs`.
5. Describe what changed and how you checked it in the pull request.

By sending a pull request you agree that your contribution is licensed under the same terms as the project: MIT or Apache-2.0, at the user's choice.

## Plugins

Plugins are WebAssembly modules built with the Rust SDK in [`sdk/`](sdk/). You do not need to change TrorPad itself to write one.

---

## Русский

Спасибо за помощь. Сообщения об ошибках, идеи и pull request'ы — всё пригодится.

### Сообщить об ошибке

Откройте [issue](https://github.com/Jozef2J/tror/issues/new/choose) и выберите «Bug report». Самые полезные сообщения содержат:

- версию TrorPad (на стартовом экране и в «Настройки → Общие») и операционную систему;
- шаги, которые приводят к ошибке, и чего вы ожидали;
- строки из панели «Журнал» около момента ошибки, если она связана с сервером, базой данных или консолью. Перед отправкой уберите пароли, имена серверов и всё личное.

Об уязвимостях не пишите в открытые issue — см. [SECURITY.md](SECURITY.md).

### Предложить возможность

Откройте issue и выберите «Feature request». Опишите задачу, которую хотите решить, а не только нужную кнопку — так проще найти самое простое решение.

### Pull request

1. Сделайте форк и ветку от `main`.
2. Соберите и проверьте: `cargo test`, `cargo clippy`. Структуру кода проверяет [deslop-please](https://github.com/deterministic-machines/deslop-please) (`cargo install deslop-please`, затем `deslop-please --allow .deslop-please.allow`): новые циклы между модулями, утечки полей и «мелкие» модули не проходят CI. Исправляйте причину, а не прячьте её; список `.deslop-please.allow` — старый долг, его можно только сокращать.
3. Одно изменение — одна тема. Пишите в стиле окружающего кода: короткие прямые функции, без комментариев в коде, общие элементы интерфейса из `src/widgets.rs`.
4. Строки интерфейса пишутся по-русски в `tr!("…")`, английский перевод добавляется в `src/i18n/en.rs`.
5. В описании pull request'а напишите, что изменилось и как вы это проверили.

Отправляя pull request, вы соглашаетесь, что ваш вклад распространяется на тех же условиях, что и проект: MIT или Apache-2.0 на выбор пользователя.
