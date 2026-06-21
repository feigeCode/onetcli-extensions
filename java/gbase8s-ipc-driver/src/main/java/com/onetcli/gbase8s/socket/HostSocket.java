package com.onetcli.gbase8s.socket;

import java.io.Closeable;
import java.io.InputStream;
import java.io.OutputStream;

public interface HostSocket extends Closeable {
    InputStream getInputStream();

    OutputStream getOutputStream();
}
