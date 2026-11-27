# Code signing policy

**English** · [Русский](#политика-подписи-кода)

Windows releases of TrorPad (`trorpad.exe`, `trorpad-setup.exe`) are being prepared for signing through the SignPath Foundation open source program. Once the project is accepted:

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

Until then the Windows files are not signed.

## What is signed

Only files built by GitHub Actions from the source code in this repository ([build.yml](.github/workflows/build.yml)). Nothing built elsewhere or taken from other projects is signed with this certificate.

## Team

| Role | Members |
| --- | --- |
| Authors (commit to the repository) | [Jozef2J](https://github.com/Jozef2J) |
| Reviewers (review changes from other contributors) | [Jozef2J](https://github.com/Jozef2J) |
| Approvers (approve every signing request) | [Jozef2J](https://github.com/Jozef2J) |

Multi-factor authentication is required for everyone with access to the repository and to SignPath.

## Privacy

TrorPad does not collect or send personal data. It connects only to servers you add yourself and, unless turned off in the settings, checks GitHub for a new version. Details: [PRIVACY.md](PRIVACY.md).

---

# Политика подписи кода

Выпуски TrorPad для Windows (`trorpad.exe`, `trorpad-setup.exe`) готовятся к подписи по программе SignPath Foundation для открытых проектов. После того как проект примут:

Бесплатная подпись кода — [SignPath.io](https://about.signpath.io/), сертификат — [SignPath Foundation](https://signpath.org/).

До этого файлы для Windows не подписаны.

## Что подписывается

Только файлы, собранные GitHub Actions из исходного кода этого репозитория ([build.yml](.github/workflows/build.yml)). Ничего собранного в другом месте или взятого из других проектов этим сертификатом не подписывается.

## Команда

| Роль | Участники |
| --- | --- |
| Авторы (вносят изменения в репозиторий) | [Jozef2J](https://github.com/Jozef2J) |
| Рецензенты (проверяют изменения других участников) | [Jozef2J](https://github.com/Jozef2J) |
| Утверждающие (подтверждают каждый запрос на подпись) | [Jozef2J](https://github.com/Jozef2J) |

Для всех, у кого есть доступ к репозиторию и к SignPath, обязательна двухфакторная аутентификация.

## Конфиденциальность

TrorPad не собирает и не отправляет личные данные. Программа подключается только к серверам, которые вы добавили сами, и, если не отключено в настройках, проверяет на GitHub наличие новой версии. Подробно: [PRIVACY.md](PRIVACY.md).
