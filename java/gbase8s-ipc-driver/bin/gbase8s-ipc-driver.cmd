@echo off
setlocal EnableDelayedExpansion

set "DIR=%~dp0"
set "JAR=%DIR%lib\gbase8s-ipc-driver.jar"
set "JDK_HOME=%GBASE8S_JDK_HOME%"
if "%JDK_HOME%"=="" set "JDK_HOME=%JAVA_HOME%"
set "DRIVER_ARGS="

:parse_args
if "%~1"=="" goto run_driver
if "%~1"=="--" (
  shift
  goto run_driver
)
if "%~1"=="--jdk-home" (
  if "%~2"=="" (
    echo Missing value for %~1 1>&2
    exit /b 1
  )
  set "JDK_HOME=%~2"
  shift
  shift
  goto parse_args
)
if "%~1"=="--java-home" (
  if "%~2"=="" (
    echo Missing value for %~1 1>&2
    exit /b 1
  )
  set "JDK_HOME=%~2"
  shift
  shift
  goto parse_args
)
echo %~1 | findstr /b /c:"--jdk-home=" >nul
if not errorlevel 1 (
  set "ARG=%~1"
  set "JDK_HOME=!ARG:--jdk-home=!"
  shift
  goto parse_args
)
echo %~1 | findstr /b /c:"--java-home=" >nul
if not errorlevel 1 (
  set "ARG=%~1"
  set "JDK_HOME=!ARG:--java-home=!"
  shift
  goto parse_args
)
goto collect_args

:collect_args
if "%~1"=="" goto run_driver
set DRIVER_ARGS=%DRIVER_ARGS% "%~1"
shift
goto collect_args

if not exist "%JAR%" (
  echo Missing driver jar: %JAR% 1>&2
  echo Run 'bash scripts/build-java-driver.sh gbase8s ^<target^>' before launching the driver. 1>&2
  exit /b 1
)

if not "%JDK_HOME%"=="" (
  set "JAVA_BIN=%JDK_HOME%\bin\java.exe"
) else (
  set "JAVA_BIN=java"
)

pushd "%DIR%" >nul
"%JAVA_BIN%" -jar "%JAR%" %DRIVER_ARGS%
set "STATUS=%ERRORLEVEL%"
popd >nul
exit /b %STATUS%
