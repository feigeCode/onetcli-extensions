package com.onetcli.gbase8s;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

final class TestFiles {
    private TestFiles() {
    }

    static void writeExecutable(File file, String content) throws IOException {
        FileOutputStream output = new FileOutputStream(file);
        try {
            output.write(content.getBytes("UTF-8"));
        } finally {
            output.close();
        }
        if (!file.setExecutable(true)) {
            throw new IOException("failed to mark executable: " + file);
        }
    }

    static void copy(InputStream input, OutputStream output) throws IOException {
        byte[] buffer = new byte[4096];
        while (true) {
            int n = input.read(buffer);
            if (n < 0) {
                return;
            }
            output.write(buffer, 0, n);
        }
    }
}
