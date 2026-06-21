package com.onetcli.gbase8s.socket;

import org.junit.Test;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

public class HostSocketConnectorTest {
    @Test
    public void socketNameUsesEnvBeforeFirstJavaArg() {
        Map<String, String> env = new LinkedHashMap<String, String>();
        env.put(HostSocketConnector.SOCKET_ENV_VAR, "from-env.sock");

        assertEquals("from-env.sock", HostSocketConnector.socketNameFromEnvOrArg(new String[]{"from-arg.sock"}, env));
    }

    @Test
    public void socketNameFallsBackToFirstJavaArg() {
        assertEquals("from-arg.sock", HostSocketConnector.socketNameFromEnvOrArg(new String[]{"from-arg.sock"}, new LinkedHashMap<String, String>()));
        assertEquals("", HostSocketConnector.socketNameFromEnvOrArg(new String[0], new LinkedHashMap<String, String>()));
    }

    @Test
    public void linuxUsesAbstractNamespaceSocketName() {
        List<HostSocketConnector.SocketTarget> targets = HostSocketConnector.resolveTargets("onet.sock", "Linux", 501);

        assertEquals(1, targets.size());
        assertTrue(targets.get(0).isAbstractNamespace());
        assertEquals("onet.sock", targets.get(0).getName());
    }

    @Test
    public void darwinUsesRunUserThenTmpPath() {
        List<HostSocketConnector.SocketTarget> targets = HostSocketConnector.resolveTargets("onet.sock", "Mac OS X", 501);

        assertEquals(2, targets.size());
        assertFalse(targets.get(0).isAbstractNamespace());
        assertEquals("/run/user/501/onet.sock", targets.get(0).getName());
        assertEquals("/tmp/onet.sock", targets.get(1).getName());
    }

    @Test
    public void windowsUsesNamedPipePath() {
        List<HostSocketConnector.SocketTarget> targets = HostSocketConnector.resolveTargets("onet.sock", "Windows 11", 501);

        assertEquals(1, targets.size());
        assertFalse(targets.get(0).isAbstractNamespace());
        assertTrue(targets.get(0).isWindowsNamedPipe());
        assertEquals("\\\\.\\pipe\\onet.sock", targets.get(0).getName());
    }

    @Test
    public void unsupportedOperatingSystemFailsClearly() {
        try {
            HostSocketConnector.resolveTargets("onet.sock", "Plan 9", 501);
        } catch (UnsupportedOperationException error) {
            assertTrue(error.getMessage().contains("not implemented"));
            return;
        }
        throw new AssertionError("expected unsupported operating system to fail");
    }
}
