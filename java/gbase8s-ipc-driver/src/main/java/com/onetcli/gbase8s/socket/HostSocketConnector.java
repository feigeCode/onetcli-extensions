package com.onetcli.gbase8s.socket;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Map;

public final class HostSocketConnector {
    public static final String SOCKET_ENV_VAR = "ONETCLI_EXT_SOCKET";

    public JnaUnixSocket.ConnectedSocket connect(String socketName) throws IOException {
        List<SocketTarget> targets = resolveTargets(socketName, System.getProperty("os.name"), JnaUnixSocket.currentUid());
        IOException lastError = null;
        for (SocketTarget target : targets) {
            try {
                return JnaUnixSocket.connect(target);
            } catch (IOException error) {
                lastError = error;
            }
        }
        if (lastError != null) {
            throw lastError;
        }
        throw new IOException("no host socket target was resolved");
    }

    public static String socketNameFromEnvOrArg(String[] args) {
        return socketNameFromEnvOrArg(args, System.getenv());
    }

    public static String socketNameFromEnvOrArg(String[] args, Map<String, String> env) {
        String fromEnv = env == null ? "" : trimToEmpty(env.get(SOCKET_ENV_VAR));
        if (!fromEnv.isEmpty()) {
            return fromEnv;
        }
        if (args != null && args.length > 0) {
            return trimToEmpty(args[0]);
        }
        return "";
    }

    public static List<SocketTarget> resolveTargets(String socketName, String osName, int uid) {
        String name = trimToEmpty(socketName);
        if (name.isEmpty()) {
            throw new IllegalArgumentException("empty host socket name");
        }
        String os = trimToEmpty(osName).toLowerCase(Locale.ENGLISH);
        List<SocketTarget> targets = new ArrayList<SocketTarget>();
        if (os.indexOf("linux") >= 0) {
            targets.add(SocketTarget.abstractNamespace(name));
            return targets;
        }
        if (os.indexOf("mac") >= 0
            || os.indexOf("darwin") >= 0
            || os.indexOf("freebsd") >= 0
            || os.indexOf("openbsd") >= 0
            || os.indexOf("netbsd") >= 0) {
            targets.add(SocketTarget.path("/run/user/" + uid + "/" + name));
            targets.add(SocketTarget.path("/tmp/" + name));
            return targets;
        }
        throw new UnsupportedOperationException("local socket is not implemented for " + osName);
    }

    private static String trimToEmpty(String value) {
        return value == null ? "" : value.trim();
    }

    public static final class SocketTarget {
        private final boolean abstractNamespace;
        private final String name;

        private SocketTarget(boolean abstractNamespace, String name) {
            this.abstractNamespace = abstractNamespace;
            this.name = name;
        }

        public static SocketTarget abstractNamespace(String name) {
            return new SocketTarget(true, name);
        }

        public static SocketTarget path(String name) {
            return new SocketTarget(false, name);
        }

        public boolean isAbstractNamespace() {
            return abstractNamespace;
        }

        public String getName() {
            return name;
        }
    }
}
