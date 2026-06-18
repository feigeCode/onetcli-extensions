package com.onetcli.gbase8s.jdbc;

import java.io.File;
import java.net.URL;
import java.net.URLClassLoader;
import java.sql.Driver;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class DriverLoader {
    public static final String JDBC_JAR_ENV = "GBASE8S_JDBC_JAR";

    private DriverLoader() {
    }

    public static Driver loadDriver(String driverClass, File workingDir) throws Exception {
        return loadDriver(driverClass, workingDir, System.getenv(JDBC_JAR_ENV));
    }

    public static Driver loadDriver(String driverClass, File workingDir, String explicitPaths) throws Exception {
        Class<?> classpathClass = findOnClasspath(driverClass);
        if (classpathClass != null) {
            return instantiateDriver(classpathClass);
        }

        List<File> jars = candidateJars(workingDir, explicitPaths);
        if (jars.isEmpty()) {
            throw new ClassNotFoundException("JDBC driver class not found and no external JDBC jars were provided: " + driverClass);
        }
        URL[] urls = new URL[jars.size()];
        for (int i = 0; i < jars.size(); i++) {
            urls[i] = jars.get(i).toURI().toURL();
        }
        URLClassLoader loader = new URLClassLoader(urls, DriverLoader.class.getClassLoader());
        Class<?> loaded = Class.forName(driverClass, true, loader);
        return instantiateDriver(loaded);
    }

    public static List<File> candidateJars(File workingDir, String explicitPaths) {
        Map<String, File> jars = new LinkedHashMap<String, File>();

        File libDir = new File(workingDir == null ? new File(".") : workingDir, "lib");
        File[] libFiles = libDir.listFiles();
        if (libFiles != null) {
            List<File> sorted = new ArrayList<File>();
            for (File file : libFiles) {
                if (isJarFile(file)) {
                    sorted.add(file);
                }
            }
            Collections.sort(sorted, new Comparator<File>() {
                @Override
                public int compare(File left, File right) {
                    return left.getName().compareTo(right.getName());
                }
            });
            for (File file : sorted) {
                putJar(jars, file);
            }
        }

        if (explicitPaths != null && !explicitPaths.trim().isEmpty()) {
            String[] parts = explicitPaths.split(java.util.regex.Pattern.quote(File.pathSeparator));
            for (String part : parts) {
                if (part != null && !part.trim().isEmpty()) {
                    File file = new File(part.trim());
                    if (isJarFile(file)) {
                        putJar(jars, file);
                    }
                }
            }
        }

        return new ArrayList<File>(jars.values());
    }

    private static Class<?> findOnClasspath(String driverClass) {
        ClassLoader context = Thread.currentThread().getContextClassLoader();
        if (context != null) {
            try {
                return Class.forName(driverClass, true, context);
            } catch (ClassNotFoundException ignored) {
                // Try the library classloader below.
            }
        }
        try {
            return Class.forName(driverClass, true, DriverLoader.class.getClassLoader());
        } catch (ClassNotFoundException ignored) {
            return null;
        }
    }

    private static Driver instantiateDriver(Class<?> driverClass) throws Exception {
        Object instance = driverClass.getDeclaredConstructor().newInstance();
        if (!(instance instanceof Driver)) {
            throw new IllegalArgumentException(driverClass.getName() + " does not implement java.sql.Driver");
        }
        return (Driver) instance;
    }

    private static boolean isJarFile(File file) {
        return file != null && file.isFile() && file.getName().toLowerCase().endsWith(".jar");
    }

    private static void putJar(Map<String, File> jars, File file) {
        jars.put(file.getAbsolutePath(), file);
    }
}
