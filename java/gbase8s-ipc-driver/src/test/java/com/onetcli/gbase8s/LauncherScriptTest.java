package com.onetcli.gbase8s;

import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class LauncherScriptTest {
    @Rule
    public TemporaryFolder temporaryFolder = new TemporaryFolder();

    @Test
    public void commandLineJdkHomeOverridesEnvironment() throws Exception {
        File cliJdk = fakeJdk("cli-jdk");
        File envJdk = fakeJdk("env-jdk");
        File javaHome = fakeJdk("java-home");

        ProcessResult result = runLauncher(
            env("GBASE8S_JDK_HOME", envJdk.getAbsolutePath(), "JAVA_HOME", javaHome.getAbsolutePath()),
            "--jdk-home",
            cliJdk.getAbsolutePath(),
            "socket-name"
        );

        assertEquals(0, result.exitCode);
        assertTrue(result.output.contains("FAKE_JAVA=" + cliJdk.getAbsolutePath()));
        assertTrue(result.output.contains("ARG=-jar"));
        assertTrue(result.output.contains("ARG=socket-name"));
    }

    @Test
    public void gbaseJdkHomeOverridesJavaHome() throws Exception {
        File envJdk = fakeJdk("env-jdk");
        File javaHome = fakeJdk("java-home");

        ProcessResult result = runLauncher(
            env("GBASE8S_JDK_HOME", envJdk.getAbsolutePath(), "JAVA_HOME", javaHome.getAbsolutePath()),
            "socket-name"
        );

        assertEquals(0, result.exitCode);
        assertTrue(result.output.contains("FAKE_JAVA=" + envJdk.getAbsolutePath()));
    }

    @Test
    public void javaHomeIsDefaultJdkHome() throws Exception {
        File javaHome = fakeJdk("java-home");

        ProcessResult result = runLauncher(env("JAVA_HOME", javaHome.getAbsolutePath()), "socket-name");

        assertEquals(0, result.exitCode);
        assertTrue(result.output.contains("FAKE_JAVA=" + javaHome.getAbsolutePath()));
    }

    @Test
    public void missingCustomJavaBinaryFailsClearly() throws Exception {
        File notAJdk = temporaryFolder.newFolder("not-a-jdk");

        ProcessResult result = runLauncher(env(), "--jdk-home", notAJdk.getAbsolutePath(), "socket-name");

        assertEquals(1, result.exitCode);
        assertTrue(result.output.contains("Java executable not found"));
    }

    private File fakeJdk(String name) throws IOException {
        File jdk = temporaryFolder.newFolder(name);
        File bin = new File(jdk, "bin");
        assertTrue(bin.mkdirs());
        File java = new File(bin, "java");
        String script = "#!/usr/bin/env sh\n"
            + "echo \"FAKE_JAVA=" + jdk.getAbsolutePath() + "\"\n"
            + "for arg in \"$@\"; do echo \"ARG=$arg\"; done\n"
            + "exit 0\n";
        TestFiles.writeExecutable(java, script);
        return jdk;
    }

    private ProcessResult runLauncher(Map<String, String> env, String... args) throws Exception {
        File lib = new File("bin/lib");
        if (!lib.isDirectory()) {
            assertTrue(lib.mkdirs());
        }
        File jar = new File(lib, "gbase8s-ipc-driver.jar");
        if (!jar.isFile()) {
            TestFiles.writeExecutable(jar, "fake jar\n");
        }
        List<String> command = new ArrayList<String>();
        command.add(new File("bin/gbase8s-ipc-driver").getAbsolutePath());
        for (String arg : args) {
            command.add(arg);
        }
        ProcessBuilder builder = new ProcessBuilder(command);
        builder.directory(new File("."));
        builder.redirectErrorStream(true);
        builder.environment().remove("GBASE8S_JDK_HOME");
        builder.environment().remove("JAVA_HOME");
        builder.environment().putAll(env);
        Process process = builder.start();
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        TestFiles.copy(process.getInputStream(), output);
        int exitCode = process.waitFor();
        return new ProcessResult(exitCode, new String(output.toByteArray(), "UTF-8"));
    }

    private Map<String, String> env(String... keyValues) {
        java.util.LinkedHashMap<String, String> env = new java.util.LinkedHashMap<String, String>();
        for (int i = 0; i < keyValues.length; i += 2) {
            env.put(keyValues[i], keyValues[i + 1]);
        }
        return env;
    }

    private static final class ProcessResult {
        private final int exitCode;
        private final String output;

        private ProcessResult(int exitCode, String output) {
            this.exitCode = exitCode;
            this.output = output;
        }
    }
}
