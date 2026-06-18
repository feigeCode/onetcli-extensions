package com.onetcli.gbase8s.jdbc;

import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

import java.io.File;
import java.sql.Driver;
import java.util.List;

import static org.junit.Assume.assumeTrue;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class DriverLoaderTest {
    @Rule
    public TemporaryFolder temporaryFolder = new TemporaryFolder();

    @Test
    public void candidateJarsIncludeSortedLibJarsAndExplicitEnvPaths() throws Exception {
        File workDir = temporaryFolder.newFolder("driver");
        File lib = new File(workDir, "lib");
        assertTrue(lib.mkdirs());
        File z = new File(lib, "z-driver.jar");
        File a = new File(lib, "a-driver.jar");
        File ignored = new File(lib, "notes.txt");
        assertTrue(z.createNewFile());
        assertTrue(a.createNewFile());
        assertTrue(ignored.createNewFile());
        File explicit = temporaryFolder.newFile("official.jar");

        List<File> jars = DriverLoader.candidateJars(
            workDir,
            explicit.getAbsolutePath() + File.pathSeparator + new File(workDir, "missing.jar").getAbsolutePath()
        );

        assertEquals(3, jars.size());
        assertEquals("a-driver.jar", jars.get(0).getName());
        assertEquals("z-driver.jar", jars.get(1).getName());
        assertEquals("official.jar", jars.get(2).getName());
    }

    @Test
    public void loadDriverUsesExistingClasspathBeforeExternalJars() throws Exception {
        Driver driver = DriverLoader.loadDriver("org.h2.Driver", temporaryFolder.newFolder("empty"), "");

        assertEquals("org.h2.Driver", driver.getClass().getName());
    }

    @Test
    public void loadDriverUsesOfficialGBase8sJarFromLibWhenPresent() throws Exception {
        File workDir = new File(".");
        File officialJar = new File(workDir, "lib/gbasedbtjdbc_3.5.0_2ZY3_1_89a58a.jar");
        assumeTrue("official GBase 8s JDBC jar is not present under lib/", officialJar.isFile());

        Driver driver = DriverLoader.loadDriver("com.gbasedbt.jdbc.Driver", workDir, "");

        assertEquals("com.gbasedbt.jdbc.Driver", driver.getClass().getName());
    }
}
