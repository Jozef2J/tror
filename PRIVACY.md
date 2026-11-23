# Privacy Policy

**English** · [Русский](#русский)

Last updated: 23 November 2026. Applies to TrorPad and other apps of the Tror project (the "software").

TrorPad is free software. It is distributed at no cost and runs entirely on your computer. There is no account, no sign-up and no server of ours that your work passes through.

## What we collect

Nothing. The software contains no analytics, no telemetry, no crash reporting and no advertising. The developers do not receive any information about you, your files or how you use the software.

## Network connections the software makes

The software connects only where you tell it to, plus one optional update check:

- **Servers you add yourself** — SFTP, FTP, FTPS, SSH, Telnet, Mosh and database servers. Data goes directly between your computer and those servers.
- **Update check** — once at startup, unless turned off in the settings, the software requests `https://api.github.com/repos/Jozef2J/tror/releases/latest` to learn the latest version number. When you choose to update, the files are downloaded from GitHub. GitHub receives the usual technical data of any web request (such as your IP address) under the [GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
- **Preview in a browser** — opens the page you chose in your browser.
- **Plugins** — a plugin can reach the network only if you grant it that permission.

## Data stored on your computer

All of this stays on your device, in the software's data folder (for example `%APPDATA%\Tror` on Windows):

- settings, recent files and window layout;
- connection profiles; passwords are kept in the operating system's credential store, or masked in `connections.json` if you choose the portable mode;
- copies of unsaved documents for crash recovery;
- backup copies of files that were overwritten on servers (the last 10 versions of each file);
- database query history;
- installed plugins and their data.

You can delete any of it at any time. The Windows uninstaller offers to remove it.

## Plugins

Plugins run in a sandbox and can only use the permissions you grant them (files, network, servers, databases). Plugins never receive your passwords. Third-party plugins are not made by us; check where a plugin comes from before you install it.

## Children

The software is a developer tool and is not directed at children. It does not knowingly collect data from anyone, including children.

## Changes

If this policy changes, the new version will be published in this file in the repository, with a new date.

## Contact

Questions: open an issue at https://github.com/Jozef2J/tror/issues.

---

# Политика конфиденциальности

<a id="русский"></a>

[English](#privacy-policy) · **Русский**

Дата обновления: 23 ноября 2026 г. Относится к TrorPad и другим программам проекта Tror (далее — «программа»).

TrorPad — бесплатная программа. Она распространяется свободно и полностью работает на вашем компьютере. Нет учётной записи, регистрации и нашего сервера, через который проходила бы ваша работа.

## Что мы собираем

Ничего. В программе нет аналитики, телеметрии, отчётов о сбоях и рекламы. Разработчики не получают никаких сведений о вас, ваших файлах и о том, как вы пользуетесь программой.

## Какие подключения делает программа

Программа подключается только туда, куда скажете вы, плюс необязательная проверка обновлений:

- **Серверы, которые вы добавили сами** — SFTP, FTP, FTPS, SSH, Telnet, Mosh и серверы баз данных. Данные идут напрямую между вашим компьютером и этими серверами.
- **Проверка обновлений** — один раз при запуске, если не выключена в настройках, программа запрашивает `https://api.github.com/repos/Jozef2J/tror/releases/latest`, чтобы узнать номер последней версии. Когда вы решаете обновиться, файлы скачиваются с GitHub. GitHub получает обычные технические данные любого веб-запроса (например, IP-адрес) по своим [правилам конфиденциальности](https://docs.github.com/ru/site-policy/privacy-policies/github-general-privacy-statement).
- **Предпросмотр в браузере** — открывает выбранную вами страницу в браузере.
- **Плагины** — плагин выходит в сеть, только если вы дали ему это разрешение.

## Что хранится на вашем компьютере

Всё это остаётся на вашем устройстве, в папке данных программы (например, `%APPDATA%\Tror` в Windows):

- настройки, недавние файлы и расположение окон;
- профили подключений; пароли — в системном хранилище учётных данных или замаскированными в `connections.json`, если выбран переносной режим;
- копии несохранённых документов для восстановления после сбоя;
- резервные копии файлов, перезаписанных на серверах (последние 10 версий каждого файла);
- история запросов к базам данных;
- установленные плагины и их данные.

Любые из этих данных можно удалить в любой момент. Деинсталлятор Windows предлагает их удалить.

## Плагины

Плагины работают в изолированной среде и пользуются только теми разрешениями, которые вы дали (файлы, сеть, серверы, базы данных). Пароли плагинам не передаются никогда. Сторонние плагины сделаны не нами — проверяйте источник плагина перед установкой.

## Дети

Программа — инструмент разработчика и не предназначена для детей. Она сознательно не собирает данные ни о ком, в том числе о детях.

## Изменения

Если политика изменится, новая версия будет опубликована в этом файле репозитория с новой датой.

## Связь

Вопросы — через issue: https://github.com/Jozef2J/tror/issues.
