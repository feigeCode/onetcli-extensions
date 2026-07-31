package com.navop.gbase8s.server;

import java.math.BigInteger;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

final class HostVersion {
    static final String MINIMUM = "0.10.0";
    private static final Pattern SEMVER = Pattern.compile(
            "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*))?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$");

    private HostVersion() {
    }

    static String incompatibility(String version) {
        Matcher matcher = SEMVER.matcher(version == null ? "" : version);
        if (!matcher.matches()) {
            return "this driver requires Navop >= " + MINIMUM + "; invalid or missing host_version `" + version + "`";
        }
        BigInteger major = new BigInteger(matcher.group(1));
        BigInteger minor = new BigInteger(matcher.group(2));
        boolean tooOld = matcher.group(4) != null
                || (major.signum() == 0 && minor.compareTo(BigInteger.TEN) < 0);
        if (tooOld) {
            return "this driver requires Navop >= " + MINIMUM + "; current host version is "
                    + version + ". Please upgrade Navop";
        }
        return null;
    }
}
