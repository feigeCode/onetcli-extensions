package oceanbase

import (
	"fmt"
	"strings"

	"github.com/helingjun/obconnector-go/internal/protocol"
)

func capabilityNames(flags uint32) string {
	known := []struct {
		bit  uint32
		name string
	}{
		{protocol.ClientLongPassword, "CLIENT_LONG_PASSWORD"},
		{protocol.ClientFoundRows, "CLIENT_FOUND_ROWS"},
		{protocol.ClientLongFlag, "CLIENT_LONG_FLAG"},
		{protocol.ClientConnectWithDB, "CLIENT_CONNECT_WITH_DB"},
		{protocol.ClientNoSchema, "CLIENT_NO_SCHEMA"},
		{protocol.ClientCompress, "CLIENT_COMPRESS"},
		{protocol.ClientODBC, "CLIENT_ODBC"},
		{protocol.ClientLocalFiles, "CLIENT_LOCAL_FILES"},
		{protocol.ClientIgnoreSpace, "CLIENT_IGNORE_SPACE"},
		{protocol.ClientProtocol41, "CLIENT_PROTOCOL_41"},
		{protocol.ClientInteractive, "CLIENT_INTERACTIVE"},
		{protocol.ClientSSL, "CLIENT_SSL"},
		{protocol.ClientIgnoreSigpipe, "CLIENT_IGNORE_SIGPIPE"},
		{protocol.ClientTransactions, "CLIENT_TRANSACTIONS"},
		{protocol.ClientReserved, "CLIENT_RESERVED"},
		{protocol.ClientSecureConnection, "CLIENT_SECURE_CONNECTION"},
		{protocol.ClientMultiStatements, "CLIENT_MULTI_STATEMENTS"},
		{protocol.ClientMultiResults, "CLIENT_MULTI_RESULTS"},
		{protocol.ClientPSMultiResults, "CLIENT_PS_MULTI_RESULTS"},
		{protocol.ClientPluginAuth, "CLIENT_PLUGIN_AUTH"},
		{protocol.ClientConnectAttrs, "CLIENT_CONNECT_ATTRS"},
		{protocol.ClientPluginAuthLenencClientData, "CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA"},
		{protocol.ClientCanHandleExpiredPasswords, "CLIENT_CAN_HANDLE_EXPIRED_PASSWORDS"},
		{protocol.ClientSessionTrack, "CLIENT_SESSION_TRACK"},
		{protocol.ClientDeprecateEOF, "CLIENT_DEPRECATE_EOF"},
		{protocol.ClientSupportOracleMode, "CLIENT_SUPPORT_ORACLE_MODE"},
		{protocol.ClientReturnHiddenRowID, "CLIENT_RETURN_HIDDEN_ROWID"},
		{protocol.ClientUseLOBLocator, "CLIENT_USE_LOB_LOCATOR"},
	}

	var out []string
	remaining := flags
	for _, item := range known {
		if flags&item.bit == 0 {
			continue
		}
		out = append(out, item.name)
		remaining &^= item.bit
	}
	if remaining != 0 {
		out = append(out, fmt.Sprintf("UNKNOWN(0x%08x)", remaining))
	}
	if len(out) == 0 {
		return "none"
	}
	return strings.Join(out, "|")
}
