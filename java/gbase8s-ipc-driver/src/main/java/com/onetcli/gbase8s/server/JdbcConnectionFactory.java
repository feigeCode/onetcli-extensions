package com.onetcli.gbase8s.server;

import com.onetcli.gbase8s.jdbc.GBase8sConfig;

import java.sql.Connection;

public interface JdbcConnectionFactory {
    Connection open(GBase8sConfig config) throws Exception;
}
