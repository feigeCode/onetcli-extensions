package com.onetcli.gbase8s.ipc;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

public final class FramedJsonTransport {
    public static final int MAX_FRAME_BYTES = 16 * 1024 * 1024;

    private final DataInputStream input;
    private final DataOutputStream output;
    private final ObjectMapper mapper;

    private FramedJsonTransport(InputStream input, OutputStream output, ObjectMapper mapper) {
        this.input = input == null ? null : new DataInputStream(input);
        this.output = output == null ? null : new DataOutputStream(output);
        this.mapper = mapper;
    }

    public static FramedJsonTransport forInput(InputStream input, ObjectMapper mapper) {
        return new FramedJsonTransport(input, null, mapper);
    }

    public static FramedJsonTransport forOutput(OutputStream output, ObjectMapper mapper) {
        return new FramedJsonTransport(null, output, mapper);
    }

    public static FramedJsonTransport forStreams(InputStream input, OutputStream output, ObjectMapper mapper) {
        return new FramedJsonTransport(input, output, mapper);
    }

    public JsonNode read() throws IOException {
        if (input == null) {
            throw new IOException("transport has no input stream");
        }
        byte[] prefix = new byte[4];
        try {
            input.readFully(prefix);
        } catch (EOFException error) {
            throw error;
        }
        long length = ((long) prefix[0] & 0xffL)
            | (((long) prefix[1] & 0xffL) << 8)
            | (((long) prefix[2] & 0xffL) << 16)
            | (((long) prefix[3] & 0xffL) << 24);
        if (length > MAX_FRAME_BYTES) {
            throw new IOException("frame length " + length + " exceeds limit " + MAX_FRAME_BYTES);
        }
        byte[] payload = new byte[(int) length];
        input.readFully(payload);
        return mapper.readTree(payload);
    }

    public void write(JsonNode message) throws IOException {
        if (output == null) {
            throw new IOException("transport has no output stream");
        }
        byte[] payload = mapper.writeValueAsBytes(message);
        if (payload.length > MAX_FRAME_BYTES) {
            throw new IOException("payload length " + payload.length + " exceeds limit " + MAX_FRAME_BYTES);
        }
        writeLittleEndianLength(payload.length);
        output.write(payload);
        output.flush();
    }

    private void writeLittleEndianLength(int length) throws IOException {
        output.write(length & 0xff);
        output.write((length >> 8) & 0xff);
        output.write((length >> 16) & 0xff);
        output.write((length >> 24) & 0xff);
    }
}
