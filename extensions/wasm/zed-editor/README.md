# Zed External Editor

This static composite extension contributes Zed as an external editor for
Navop SFTP remote files. The host application owns the SFTP connection,
temporary file, change watcher, conflict prompt, and upload workflow.

The extension contains no executable code and receives no credentials. On
macOS, the standard Zed executable is used for availability checks and files are
delivered through LaunchServices. Linux uses the `zed` PATH command, while
Windows uses the standard `%ProgramFiles%\Zed\Zed.exe` installation path.

After installation, right-click a remote file and choose **Edit With Zed**. For
a non-standard installation, configure the executable path in Navop Settings
under **Remote File Editor**.
