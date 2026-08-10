package oceanbase

import (
	"strconv"

	"github.com/helingjun/obconnector-go/internal/protocol"
)

const (
	obClientCapOBProtocolV2                uint64 = 1 << 8
	obClientCapFullLinkTrace               uint64 = 1 << 14
	obClientCapProxyNewExtraInfo           uint64 = 1 << 15
	obClientCapProxyFullLinkTraceShowTrace uint64 = 1 << 18
	obClientCapOBLOBLocatorV2              uint64 = 1 << 0
	obClientCapSupportJDBCBinaryDouble     uint64 = 1 << 2
)

func presetCapabilities(preset string) uint32 {
	switch preset {
	case "oboracle", "obclient", "libobclient", "connector-c":
		return protocol.ClientMultiStatements |
			protocol.ClientPSMultiResults |
			protocol.ClientLocalFiles |
			protocol.ClientInteractive |
			protocol.ClientSessionTrack |
			protocol.ClientSupportOracleMode
	default:
		return protocol.ClientSupportOracleMode
	}
}

func formatUint64(v uint64) string {
	return strconv.FormatUint(v, 10)
}

func presetAttributes(preset string) map[string]string {
	switch preset {
	case "oboracle", "libobclient", "connector-c":
		return obConnectorCAttributes()
	case "obclient":
		attrs := obConnectorCAttributes()
		attrs["program_name"] = "obclient"
		return attrs
	case "connector-j":
		return map[string]string{
			"_client_name":        "OceanBase Connector/J",
			"_client_version":     "2.4.14",
			"__ob_client_name":    "OceanBase Connector/J",
			"__ob_client_version": "2.4.14",
			"__mysql_client_type": "__ob_jdbc_client",
			"_runtime_vendor":     "Go",
			"program_name":        "obconnector-go",
		}
	default:
		return obConnectorCAttributes()
	}
}

func obConnectorCAttributes() map[string]string {
	proxyCaps := obClientCapOBProtocolV2 |
		obClientCapFullLinkTrace |
		obClientCapProxyNewExtraInfo |
		obClientCapProxyFullLinkTraceShowTrace
	attrCaps := obClientCapOBLOBLocatorV2 | obClientCapSupportJDBCBinaryDouble

	return map[string]string{
		"_client_name":                          "OceanBase Connector/Go",
		"_client_version":                       Version,
		"__mysql_client_type":                   "__ob_go_client",
		"__ob_client_name":                      "OceanBase Connector/Go",
		"__ob_client_version":                   Version,
		"ob_capability_flag":                    formatUint64(proxyCaps),
		"__proxy_capability_flag":               formatUint64(proxyCaps),
		"__ob_client_attribute_capability_flag": formatUint64(attrCaps),
	}
}
