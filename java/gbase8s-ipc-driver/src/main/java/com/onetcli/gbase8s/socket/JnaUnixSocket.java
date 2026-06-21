package com.onetcli.gbase8s.socket;

import com.sun.jna.Library;
import com.sun.jna.Memory;
import com.sun.jna.Native;
import com.sun.jna.Platform;
import com.sun.jna.Pointer;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;

public final class JnaUnixSocket {
    private static final int AF_UNIX = 1;
    private static final int SOCK_STREAM = 1;
    private static final int EINTR = 4;
    private static final int LINUX_SUN_PATH_BYTES = 108;
    private static final int BSD_SUN_PATH_BYTES = 104;

    private JnaUnixSocket() {
    }

    public static int currentUid() {
        return LibC.INSTANCE.getuid();
    }

    public static ConnectedSocket connect(HostSocketConnector.SocketTarget target) throws IOException {
        int fd = LibC.INSTANCE.socket(AF_UNIX, SOCK_STREAM, 0);
        if (fd < 0) {
            throw ioException("socket", Native.getLastError());
        }
        boolean connected = false;
        try {
            SockAddr address = SockAddr.fromTarget(target);
            int rc = LibC.INSTANCE.connect(fd, address.pointer, address.length);
            if (rc < 0) {
                throw ioException("connect " + target.getName(), Native.getLastError());
            }
            connected = true;
            return new ConnectedSocket(fd);
        } finally {
            if (!connected) {
                LibC.INSTANCE.close(fd);
            }
        }
    }

    private static IOException ioException(String operation, int errno) {
        String message = LibC.INSTANCE.strerror(errno);
        return new IOException(operation + " failed: " + message + " (errno " + errno + ")");
    }

    private interface LibC extends Library {
        LibC INSTANCE = Native.load(Platform.C_LIBRARY_NAME, LibC.class);

        int socket(int domain, int type, int protocol);

        int connect(int fd, Pointer address, int addressLength);

        int read(int fd, byte[] buffer, int count);

        int write(int fd, byte[] buffer, int count);

        int close(int fd);

        int getuid();

        String strerror(int errno);
    }

    private static final class SockAddr {
        private final Memory pointer;
        private final int length;

        private SockAddr(Memory pointer, int length) {
            this.pointer = pointer;
            this.length = length;
        }

        private static SockAddr fromTarget(HostSocketConnector.SocketTarget target) throws IOException {
            if (Platform.isLinux()) {
                return linuxSockAddr(target);
            }
            return bsdSockAddr(target);
        }

        private static SockAddr linuxSockAddr(HostSocketConnector.SocketTarget target) throws IOException {
            byte[] name = target.getName().getBytes(StandardCharsets.UTF_8);
            int offset = 2;
            byte[] path;
            int length;
            if (target.isAbstractNamespace()) {
                path = new byte[name.length + 1];
                System.arraycopy(name, 0, path, 1, name.length);
                length = offset + path.length;
            } else {
                path = name;
                length = offset + path.length + 1;
            }
            if (path.length > LINUX_SUN_PATH_BYTES) {
                throw new IOException("unix socket path is too long: " + target.getName());
            }
            Memory memory = new Memory(length);
            memory.clear();
            memory.setShort(0, (short) AF_UNIX);
            memory.write(offset, path, 0, path.length);
            return new SockAddr(memory, length);
        }

        private static SockAddr bsdSockAddr(HostSocketConnector.SocketTarget target) throws IOException {
            if (target.isAbstractNamespace()) {
                throw new IOException("abstract unix sockets are only supported on Linux");
            }
            byte[] path = target.getName().getBytes(StandardCharsets.UTF_8);
            if (path.length > BSD_SUN_PATH_BYTES) {
                throw new IOException("unix socket path is too long: " + target.getName());
            }
            int length = 2 + path.length + 1;
            Memory memory = new Memory(length);
            memory.clear();
            memory.setByte(0, (byte) length);
            memory.setByte(1, (byte) AF_UNIX);
            memory.write(2, path, 0, path.length);
            return new SockAddr(memory, length);
        }
    }

    public static final class ConnectedSocket implements HostSocket {
        private final int fd;
        private final InputStream inputStream = new SocketInputStream();
        private final OutputStream outputStream = new SocketOutputStream();
        private boolean closed;

        private ConnectedSocket(int fd) {
            this.fd = fd;
        }

        public InputStream getInputStream() {
            return inputStream;
        }

        public OutputStream getOutputStream() {
            return outputStream;
        }

        @Override
        public synchronized void close() throws IOException {
            if (closed) {
                return;
            }
            closed = true;
            int rc = LibC.INSTANCE.close(fd);
            if (rc < 0) {
                throw ioException("close", Native.getLastError());
            }
        }

        private synchronized void ensureOpen() throws IOException {
            if (closed) {
                throw new IOException("socket is closed");
            }
        }

        private final class SocketInputStream extends InputStream {
            @Override
            public int read() throws IOException {
                byte[] one = new byte[1];
                int n = read(one, 0, 1);
                return n < 0 ? -1 : one[0] & 0xff;
            }

            @Override
            public int read(byte[] buffer, int offset, int length) throws IOException {
                if (buffer == null) {
                    throw new NullPointerException("buffer");
                }
                if (offset < 0 || length < 0 || length > buffer.length - offset) {
                    throw new IndexOutOfBoundsException();
                }
                if (length == 0) {
                    return 0;
                }
                ensureOpen();
                byte[] target = offset == 0 && length == buffer.length ? buffer : new byte[length];
                while (true) {
                    int rc = LibC.INSTANCE.read(fd, target, length);
                    if (rc > 0) {
                        if (target != buffer) {
                            System.arraycopy(target, 0, buffer, offset, rc);
                        }
                        return rc;
                    }
                    if (rc == 0) {
                        return -1;
                    }
                    int errno = Native.getLastError();
                    if (errno == EINTR) {
                        continue;
                    }
                    throw ioException("read", errno);
                }
            }

            @Override
            public void close() throws IOException {
                ConnectedSocket.this.close();
            }
        }

        private final class SocketOutputStream extends OutputStream {
            @Override
            public void write(int value) throws IOException {
                byte[] one = new byte[]{(byte) value};
                write(one, 0, 1);
            }

            @Override
            public void write(byte[] buffer, int offset, int length) throws IOException {
                if (buffer == null) {
                    throw new NullPointerException("buffer");
                }
                if (offset < 0 || length < 0 || length > buffer.length - offset) {
                    throw new IndexOutOfBoundsException();
                }
                int remaining = length;
                int position = offset;
                while (remaining > 0) {
                    ensureOpen();
                    byte[] chunk = new byte[remaining];
                    System.arraycopy(buffer, position, chunk, 0, remaining);
                    int rc = LibC.INSTANCE.write(fd, chunk, remaining);
                    if (rc > 0) {
                        position += rc;
                        remaining -= rc;
                        continue;
                    }
                    if (rc == 0) {
                        throw new IOException("socket write returned 0 bytes");
                    }
                    int errno = Native.getLastError();
                    if (errno == EINTR) {
                        continue;
                    }
                    throw ioException("write", errno);
                }
            }

            @Override
            public void close() throws IOException {
                ConnectedSocket.this.close();
            }
        }
    }
}
