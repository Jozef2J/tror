# Security Policy

**English** · [Русский](#русский)

## Supported versions

Only the latest release receives fixes. TrorPad offers updates itself, so please update before reporting.

## Reporting a vulnerability

Please do not describe the problem in a public issue. Report it privately: open the [Security tab](https://github.com/Jozef2J/tror/security) and press "Report a vulnerability". If that button is not there, open an issue titled "Security contact" without any details, and the maintainer will reply with a private channel.

Useful details: the version, the operating system, what an attacker can do and the steps to reproduce it. You will get an answer within a week. When the fix is released, the report is published with credit to you, unless you prefer otherwise.

## What matters most

TrorPad stores server and database passwords and connects to remote machines, so these areas are the most sensitive:

- storage of passwords (the system credential store, or the profiles file in portable mode);
- SSH host key checking, SFTP/FTPS/TLS connections and SSH tunnels;
- WebAssembly plugins: a plugin must not get access beyond the permissions the user granted, and never gets passwords;
- the update check and the installer.

---

## Русский

### Поддерживаемые версии

Исправления выходят только для последней версии. TrorPad сам предлагает обновления — пожалуйста, обновитесь перед сообщением.

### Как сообщить об уязвимости

Не описывайте проблему в открытом issue. Сообщите закрыто: вкладка [Security](https://github.com/Jozef2J/tror/security) → «Report a vulnerability». Если такой кнопки нет, откройте issue с заголовком «Security contact» без подробностей — сопровождающий ответит и предложит закрытый канал.

Укажите версию, систему, что может сделать злоумышленник и как это воспроизвести. Ответ — в течение недели. После выхода исправления сообщение публикуется с благодарностью вам, если вы не против.

### Самое важное

TrorPad хранит пароли от серверов и баз и подключается к удалённым машинам, поэтому особенно важны: хранение паролей, проверка ключей SSH, соединения SFTP/FTPS/TLS и SSH-туннели, плагины WebAssembly (плагин не получает больше прав, чем разрешил пользователь, и никогда не получает пароли), проверка обновлений и установщик.
