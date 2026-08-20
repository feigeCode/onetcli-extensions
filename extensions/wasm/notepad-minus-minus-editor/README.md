# Notepad-- External Editor

This static composite extension contributes Notepad-- as an external editor for
Navop SFTP remote files. The host application owns the SFTP connection,
temporary file, change watcher, conflict prompt, and upload workflow.

The extension contains no executable code and receives no credentials. On
macOS, it declares the standard Notepad-- executable for availability checks,
then asks Navop to deliver `{file}` through macOS LaunchServices. Windows uses
the standard `%ProgramFiles%\Notepad--\Notepad--.exe` installation path.

After installation, right-click a remote file and choose **Edit With Notepad--**.
For a non-standard installation, configure the executable path in Navop
Settings under **Remote File Editor**.
