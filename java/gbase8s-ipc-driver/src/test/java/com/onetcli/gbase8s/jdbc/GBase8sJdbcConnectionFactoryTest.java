package com.onetcli.gbase8s.jdbc;

import org.junit.Test;

import java.io.File;
import java.sql.Connection;
import java.sql.Driver;
import java.sql.DriverManager;
import java.sql.DriverPropertyInfo;
import java.sql.SQLException;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Properties;
import java.util.logging.Logger;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class GBase8sJdbcConnectionFactoryTest {
    @Test
    public void openUsesOfficialUrlAndCredentialProperties() throws Exception {
        RecordingDriver.lastUrl = null;
        RecordingDriver.lastProperties = null;

        GBase8sConfig config = GBase8sConfig.fromWire(configFor(RecordingDriver.class.getName()));
        GBase8sJdbcConnectionFactory factory = new GBase8sJdbcConnectionFactory(new File("."));

        Connection connection = factory.open(config);
        try {
            assertEquals(
                "jdbc:gbasedbt-sqli://127.0.0.1:11811/stores:GBASEDBTSERVER=gbase01;PROTOCOL=onsoctcp;",
                RecordingDriver.lastUrl
            );
            assertEquals("gbasedbt", RecordingDriver.lastProperties.getProperty("user"));
            assertEquals("secret", RecordingDriver.lastProperties.getProperty("password"));
        } finally {
            connection.close();
        }
    }

    @Test
    public void openFailsWhenDriverDoesNotAcceptUrl() throws Exception {
        GBase8sConfig config = GBase8sConfig.fromWire(configFor(NullDriver.class.getName()));
        GBase8sJdbcConnectionFactory factory = new GBase8sJdbcConnectionFactory(new File("."));

        try {
            factory.open(config);
        } catch (SQLException error) {
            assertTrue(error.getMessage().contains("did not accept JDBC URL"));
            return;
        }
        throw new AssertionError("expected null driver connection to fail");
    }

    private static Map<String, Object> configFor(String driverClass) {
        Map<String, Object> raw = new LinkedHashMap<String, Object>();
        raw.put("host", "127.0.0.1");
        raw.put("username", "gbasedbt");
        raw.put("password", "secret");
        raw.put("database", "stores");
        raw.put("driver_class", driverClass);
        Map<String, Object> extra = new LinkedHashMap<String, Object>();
        extra.put("GBASEDBTSERVER", "gbase01");
        extra.put("PROTOCOL", "onsoctcp");
        raw.put("extra_params", extra);
        return raw;
    }

    public static final class RecordingDriver implements Driver {
        private static String lastUrl;
        private static Properties lastProperties;

        @Override
        public Connection connect(String url, Properties info) throws SQLException {
            lastUrl = url;
            lastProperties = info;
            return DriverManager.getConnection("jdbc:h2:mem:gbase8s_factory_recording");
        }

        @Override
        public boolean acceptsURL(String url) {
            return true;
        }

        @Override
        public DriverPropertyInfo[] getPropertyInfo(String url, Properties info) {
            return new DriverPropertyInfo[0];
        }

        @Override
        public int getMajorVersion() {
            return 1;
        }

        @Override
        public int getMinorVersion() {
            return 0;
        }

        @Override
        public boolean jdbcCompliant() {
            return false;
        }

        @Override
        public Logger getParentLogger() {
            return Logger.getGlobal();
        }
    }

    public static final class NullDriver implements Driver {
        @Override
        public Connection connect(String url, Properties info) {
            return null;
        }

        @Override
        public boolean acceptsURL(String url) {
            return false;
        }

        @Override
        public DriverPropertyInfo[] getPropertyInfo(String url, Properties info) {
            return new DriverPropertyInfo[0];
        }

        @Override
        public int getMajorVersion() {
            return 1;
        }

        @Override
        public int getMinorVersion() {
            return 0;
        }

        @Override
        public boolean jdbcCompliant() {
            return false;
        }

        @Override
        public Logger getParentLogger() {
            return Logger.getGlobal();
        }
    }
}
