package com.onetcli.gbase8s.ipc;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.Test;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.EOFException;
import java.io.IOException;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class FramedJsonTransportTest {
    private final ObjectMapper mapper = new ObjectMapper();

    @Test
    public void writeMessagePrefixesJsonWithLittleEndianLength() throws Exception {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        FramedJsonTransport transport = FramedJsonTransport.forOutput(out, mapper);

        transport.write(mapper.readTree("{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}"));

        byte[] bytes = out.toByteArray();
        int len = (bytes[0] & 0xff)
            | ((bytes[1] & 0xff) << 8)
            | ((bytes[2] & 0xff) << 16)
            | ((bytes[3] & 0xff) << 24);
        assertEquals(bytes.length - 4, len);

        String json = new String(bytes, 4, len, "UTF-8");
        assertEquals("2.0", mapper.readTree(json).get("jsonrpc").asText());
    }

    @Test
    public void readMessageParsesLittleEndianFrame() throws Exception {
        byte[] payload = "{\"jsonrpc\":\"2.0\",\"id\":\"abc\",\"method\":\"init\"}".getBytes("UTF-8");
        ByteArrayOutputStream raw = new ByteArrayOutputStream();
        raw.write(payload.length & 0xff);
        raw.write((payload.length >> 8) & 0xff);
        raw.write((payload.length >> 16) & 0xff);
        raw.write((payload.length >> 24) & 0xff);
        raw.write(payload);

        FramedJsonTransport transport = FramedJsonTransport.forInput(
            new ByteArrayInputStream(raw.toByteArray()),
            mapper
        );

        JsonNode node = transport.read();

        assertEquals("abc", node.get("id").asText());
        assertEquals("init", node.get("method").asText());
    }

    @Test
    public void readMessageRejectsFrameAboveLimit() throws Exception {
        ByteArrayOutputStream raw = new ByteArrayOutputStream();
        int tooLarge = FramedJsonTransport.MAX_FRAME_BYTES + 1;
        raw.write(tooLarge & 0xff);
        raw.write((tooLarge >> 8) & 0xff);
        raw.write((tooLarge >> 16) & 0xff);
        raw.write((tooLarge >> 24) & 0xff);

        FramedJsonTransport transport = FramedJsonTransport.forInput(
            new ByteArrayInputStream(raw.toByteArray()),
            mapper
        );

        try {
            transport.read();
        } catch (IOException error) {
            assertTrue(error.getMessage().contains("exceeds limit"));
            return;
        }
        throw new AssertionError("expected oversized frame to fail");
    }

    @Test(expected = EOFException.class)
    public void readMessagePropagatesEof() throws Exception {
        FramedJsonTransport transport = FramedJsonTransport.forInput(
            new ByteArrayInputStream(new byte[0]),
            mapper
        );

        transport.read();
    }
}
