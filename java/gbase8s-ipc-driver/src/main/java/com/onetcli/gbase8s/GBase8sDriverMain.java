package com.onetcli.gbase8s;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.onetcli.gbase8s.ipc.FramedJsonTransport;
import com.onetcli.gbase8s.jdbc.GBase8sJdbcConnectionFactory;
import com.onetcli.gbase8s.server.GBase8sIpcServer;
import com.onetcli.gbase8s.socket.HostSocketConnector;
import com.onetcli.gbase8s.socket.JnaUnixSocket;

import java.io.EOFException;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

public final class GBase8sDriverMain {
    private GBase8sDriverMain() {
    }

    public static void main(String[] args) throws Exception {
        String socketName = HostSocketConnector.socketNameFromEnvOrArg(args);
        if (socketName.isEmpty()) {
            throw new IllegalArgumentException("missing ONETCLI_EXT_SOCKET or first socket name argument");
        }

        File workingDir = new File(System.getProperty("user.dir", "."));
        GBase8sIpcServer server = new GBase8sIpcServer(new GBase8sJdbcConnectionFactory(workingDir));
        JnaUnixSocket.ConnectedSocket socket = new HostSocketConnector().connect(socketName);
        try {
            serve(socket.getInputStream(), socket.getOutputStream(), server);
        } finally {
            socket.close();
        }
    }

    public static void serve(InputStream input, OutputStream output, GBase8sIpcServer server) throws IOException {
        ObjectMapper mapper = new ObjectMapper();
        FramedJsonTransport transport = FramedJsonTransport.forStreams(input, output, mapper);
        while (true) {
            JsonNode request;
            try {
                request = transport.read();
            } catch (EOFException eof) {
                return;
            }

            boolean shutdown = isShutdown(request);
            if (!hasId(request)) {
                if (shutdown) {
                    return;
                }
                continue;
            }

            JsonNode response = server.handle(request);
            transport.write(response);
            if (shutdown) {
                return;
            }
        }
    }

    private static boolean hasId(JsonNode request) {
        return request != null && request.has("id");
    }

    private static boolean isShutdown(JsonNode request) {
        return request != null && "shutdown".equals(request.path("method").asText(""));
    }
}
