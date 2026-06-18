package com.onetcli.gbase8s.server;

public final class ProtocolError {
    public static final int PARSE_ERROR = -32700;
    public static final int INVALID_REQUEST = -32600;
    public static final int METHOD_NOT_FOUND = -32601;
    public static final int INVALID_PARAMS = -32602;
    public static final int INTERNAL_ERROR = -32603;
    public static final int NOT_INITIALIZED = -32001;
    public static final int UNKNOWN_CONN_ID = -32007;
    public static final int UNKNOWN_CURSOR_ID = -32008;
    public static final int CONNECTION_FAILED = -33001;
    public static final int SQL_SYNTAX = -34001;

    private ProtocolError() {
    }
}
