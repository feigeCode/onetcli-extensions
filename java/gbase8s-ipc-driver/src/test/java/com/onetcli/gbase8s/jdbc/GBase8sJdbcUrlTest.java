package com.onetcli.gbase8s.jdbc;

import org.junit.Test;

import java.util.LinkedHashMap;
import java.util.Map;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class GBase8sJdbcUrlTest {
    @Test
    public void configUsesGBaseDefaultsAndNestedExtraParams() {
        GBase8sConfig config = GBase8sConfig.fromWire(validWireConfig());

        assertEquals("127.0.0.1", config.getHost());
        assertEquals(11811, config.getPort());
        assertEquals("gbasedbt", config.getUsername());
        assertEquals("secret", config.getPassword());
        assertEquals("stores", config.getDatabase());
        assertEquals("com.gbasedbt.jdbc.Driver", config.getDriverClass());
        assertEquals("gbase01", config.getExtraParams().get("GBASEDBTSERVER"));
        assertEquals("onsoctcp", config.getExtraParams().get("PROTOCOL"));
    }

    @Test
    public void configAcceptsFlatExtraParamKeysAndDriverOverride() {
        Map<String, Object> raw = validWireConfig();
        raw.remove("extra_params");
        raw.put("extra_params.GBASEDBTSERVER", "gbaseserver");
        raw.put("extra_params.PROTOCOL", "onsoctcp");
        raw.put("driver_class", "example.Driver");
        raw.put("port", "19088");

        GBase8sConfig config = GBase8sConfig.fromWire(raw);

        assertEquals(19088, config.getPort());
        assertEquals("example.Driver", config.getDriverClass());
        assertEquals("gbaseserver", config.getExtraParams().get("GBASEDBTSERVER"));
    }

    @Test
    public void jdbcUrlUsesOfficialGBase8sSqliFormatWithSortedProperties() {
        Map<String, Object> raw = validWireConfig();
        @SuppressWarnings("unchecked")
        Map<String, Object> extra = (Map<String, Object>) raw.get("extra_params");
        extra.put("CLIENT_LOCALE", "zh_cn.utf8");
        extra.put("DB_LOCALE", "zh_cn.utf8");

        GBase8sConfig config = GBase8sConfig.fromWire(raw);

        assertEquals(
            "jdbc:gbasedbt-sqli://127.0.0.1:11811/stores:CLIENT_LOCALE=zh_cn.utf8;DB_LOCALE=zh_cn.utf8;GBASEDBTSERVER=gbase01;PROTOCOL=onsoctcp;",
            GBase8sJdbcUrl.build(config)
        );
    }

    @Test
    public void missingRequiredFieldsReturnClearErrors() {
        assertInvalid(missing("host"), "host");
        assertInvalid(missing("username"), "username");
        assertInvalid(missing("database"), "database");
        assertInvalid(missingExtra("GBASEDBTSERVER"), "GBASEDBTSERVER");
        assertInvalid(missingExtra("PROTOCOL"), "PROTOCOL");
    }

    @Test
    public void jdbcUrlRejectsPropertyInjectionCharacters() {
        Map<String, Object> raw = validWireConfig();
        @SuppressWarnings("unchecked")
        Map<String, Object> extra = (Map<String, Object>) raw.get("extra_params");
        extra.put("BAD", "a;b");

        try {
            GBase8sJdbcUrl.build(GBase8sConfig.fromWire(raw));
        } catch (IllegalArgumentException error) {
            assertTrue(error.getMessage().contains("BAD"));
            return;
        }
        throw new AssertionError("expected invalid JDBC property to fail");
    }

    private static Map<String, Object> validWireConfig() {
        Map<String, Object> raw = new LinkedHashMap<String, Object>();
        raw.put("host", "127.0.0.1");
        raw.put("username", "gbasedbt");
        raw.put("password", "secret");
        raw.put("database", "stores");
        Map<String, Object> extra = new LinkedHashMap<String, Object>();
        extra.put("GBASEDBTSERVER", "gbase01");
        extra.put("PROTOCOL", "onsoctcp");
        raw.put("extra_params", extra);
        return raw;
    }

    private static Map<String, Object> missing(String key) {
        Map<String, Object> raw = validWireConfig();
        raw.remove(key);
        return raw;
    }

    private static Map<String, Object> missingExtra(String key) {
        Map<String, Object> raw = validWireConfig();
        @SuppressWarnings("unchecked")
        Map<String, Object> extra = (Map<String, Object>) raw.get("extra_params");
        extra.remove(key);
        return raw;
    }

    private static void assertInvalid(Map<String, Object> raw, String field) {
        try {
            GBase8sConfig.fromWire(raw);
        } catch (IllegalArgumentException error) {
            assertTrue(error.getMessage().contains(field));
            return;
        }
        throw new AssertionError("expected missing " + field + " to fail");
    }
}
