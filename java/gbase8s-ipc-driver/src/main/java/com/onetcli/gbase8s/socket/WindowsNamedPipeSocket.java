package com.onetcli.gbase8s.socket;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.RandomAccessFile;

public final class WindowsNamedPipeSocket implements HostSocket {
    private final RandomAccessFile pipe;
    private final InputStream inputStream = new PipeInputStream();
    private final OutputStream outputStream = new PipeOutputStream();
    private boolean closed;

    private WindowsNamedPipeSocket(RandomAccessFile pipe) {
        this.pipe = pipe;
    }

    public static WindowsNamedPipeSocket connect(String pipePath) throws IOException {
        return new WindowsNamedPipeSocket(new RandomAccessFile(pipePath, "rw"));
    }

    @Override
    public InputStream getInputStream() {
        return inputStream;
    }

    @Override
    public OutputStream getOutputStream() {
        return outputStream;
    }

    @Override
    public synchronized void close() throws IOException {
        if (closed) {
            return;
        }
        closed = true;
        pipe.close();
    }

    private synchronized void ensureOpen() throws IOException {
        if (closed) {
            throw new IOException("socket is closed");
        }
    }

    private final class PipeInputStream extends InputStream {
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
            synchronized (WindowsNamedPipeSocket.this) {
                ensureOpen();
                return pipe.read(buffer, offset, length);
            }
        }

        @Override
        public void close() throws IOException {
            WindowsNamedPipeSocket.this.close();
        }
    }

    private final class PipeOutputStream extends OutputStream {
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
            synchronized (WindowsNamedPipeSocket.this) {
                ensureOpen();
                pipe.write(buffer, offset, length);
            }
        }

        @Override
        public void flush() throws IOException {
            synchronized (WindowsNamedPipeSocket.this) {
                ensureOpen();
            }
        }

        @Override
        public void close() throws IOException {
            WindowsNamedPipeSocket.this.close();
        }
    }
}
