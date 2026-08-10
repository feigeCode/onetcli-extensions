package protocol

const (
	ClientLongPassword               uint32 = 1 << 0
	ClientFoundRows                  uint32 = 1 << 1
	ClientLongFlag                   uint32 = 1 << 2
	ClientConnectWithDB              uint32 = 1 << 3
	ClientNoSchema                   uint32 = 1 << 4
	ClientCompress                   uint32 = 1 << 5
	ClientODBC                       uint32 = 1 << 6
	ClientLocalFiles                 uint32 = 1 << 7
	ClientIgnoreSpace                uint32 = 1 << 8
	ClientProtocol41                 uint32 = 1 << 9
	ClientInteractive                uint32 = 1 << 10
	ClientSSL                        uint32 = 1 << 11
	ClientIgnoreSigpipe              uint32 = 1 << 12
	ClientTransactions               uint32 = 1 << 13
	ClientReserved                   uint32 = 1 << 14
	ClientSecureConnection           uint32 = 1 << 15
	ClientMultiStatements            uint32 = 1 << 16
	ClientMultiResults               uint32 = 1 << 17
	ClientPSMultiResults             uint32 = 1 << 18
	ClientPluginAuth                 uint32 = 1 << 19
	ClientConnectAttrs               uint32 = 1 << 20
	ClientPluginAuthLenencClientData uint32 = 1 << 21
	ClientCanHandleExpiredPasswords  uint32 = 1 << 22
	ClientSessionTrack               uint32 = 1 << 23
	ClientDeprecateEOF               uint32 = 1 << 24
	ClientSupportOracleMode          uint32 = 1 << 27
	ClientReturnHiddenRowID          uint32 = 1 << 28
	ClientUseLOBLocator              uint32 = 1 << 29

	// OceanBase specific capabilities (64-bit space, represented as uint64)
	OBCapPartitionTable             uint64 = 1
	OBCapChangeUser                 uint64 = 1 << 1
	OBCapReadWeak                   uint64 = 1 << 2
	OBCapChecksum                   uint64 = 1 << 3
	OBCapSafeWeakRead               uint64 = 1 << 4
	OBCapPriorityHit                uint64 = 1 << 5
	OBCapChecksumSwitch             uint64 = 1 << 6
	OBCapOcjEnableExtraOkPacket     uint64 = 1 << 7
	OBCapOBProtocolV2               uint64 = 1 << 8
	OBCapExtraOkPacketForStatistics uint64 = 1 << 9
	OBCapAbundantFeedback           uint64 = 1 << 10
	OBCapPLRoute                    uint64 = 1 << 11
	OBCapProxyReroute               uint64 = 1 << 12
	OBCapProxySessionSync           uint64 = 1 << 13
	OBCapFullLinkTrace              uint64 = 1 << 14
	OBCapNewExtraInfo               uint64 = 1 << 15
	OBCapProxySessionVarSync        uint64 = 1 << 16
	OBCapProxyWeakStaleFeedback     uint64 = 1 << 17
	OBCapFullLinkTraceShowTrace     uint64 = 1 << 18
	OBCapLocalFiles                 uint64 = 1 << 20

	OBClientCapLobLocatorV2      uint64 = 1
	OBCapUseNewResultsetMetadata uint64 = 1 << 2

	DefaultMaxPacketSize             uint32 = 1 << 24
	DefaultCollationUTF8MB4GeneralCI byte   = 45
	DefaultCollationUTF8MB4Bin       byte   = 46
	ComQuit                          byte   = 0x01
	ComQuery                         byte   = 0x03
	ComStmtPrepare                   byte   = 0x16
	ComStmtExecute                   byte   = 0x17
	ComStmtClose                     byte   = 0x19
	ComStmtReset                     byte   = 0x1a
	ComStmtBulkExecute               byte   = 0xfa
	ComPing                          byte   = 0x0e
	OKPacket                         byte   = 0x00
	ErrPacket                        byte   = 0xff
	EOFPacket                        byte   = 0xfe
	NullColumn                       byte   = 0xfb
)

const (
	ServerSessionStateChanged uint16 = 0x4000
	ServerPSOutParams         uint16 = 0x1000
	ServerMoreResultsExists   uint16 = 0x0008
)

const (
	ColumnTypeDecimal    byte = 0x00
	ColumnTypeTiny       byte = 0x01
	ColumnTypeShort      byte = 0x02
	ColumnTypeLong       byte = 0x03
	ColumnTypeFloat      byte = 0x04
	ColumnTypeDouble     byte = 0x05
	ColumnTypeNull       byte = 0x06
	ColumnTypeTimestamp  byte = 0x07
	ColumnTypeLongLong   byte = 0x08
	ColumnTypeInt24      byte = 0x09
	ColumnTypeDate       byte = 0x0a
	ColumnTypeTime       byte = 0x0b
	ColumnTypeDateTime   byte = 0x0c
	ColumnTypeYear       byte = 0x0d
	ColumnTypeVarChar    byte = 0x0f
	ColumnTypeBit        byte = 0x10
	ColumnTypeJSON       byte = 0xf5
	ColumnTypeNewDecimal byte = 0xf6
	ColumnTypeEnum       byte = 0xf7
	ColumnTypeSet        byte = 0xf8
	ColumnTypeTinyBlob   byte = 0xf9
	ColumnTypeMediumBlob byte = 0xfa
	ColumnTypeLongBlob   byte = 0xfb
	ColumnTypeBlob       byte = 0xfc
	ColumnTypeVarString  byte = 0xfd
	ColumnTypeString     byte = 0xfe
	ColumnTypeGeometry   byte = 0xff

	// Oracle-specific types from OB JDBC
	ColumnTypeOracleNumber        byte = 0x03 // Same as LONG, but used for NUMBER in Oracle mode
	ColumnTypeOracleBinaryFloat   byte = 0x04 // Same as FLOAT, but used for BINARY_FLOAT
	ColumnTypeOracleBinaryDouble  byte = 0x05 // Same as DOUBLE, but used for BINARY_DOUBLE
	ColumnTypeOracleTimestampTZ   byte = 0xc8 // 200
	ColumnTypeOracleTimestampLTZ  byte = 0xc9 // 201
	ColumnTypeOracleTimestampNano byte = 0xca // 202
	ColumnTypeOracleRaw           byte = 0xcb // 203
	ColumnTypeOracleIntervalYM    byte = 0xcc // 204
	ColumnTypeOracleIntervalDS    byte = 0xcd // 205
	ColumnTypeOracleNumberFloat   byte = 0xce // 206
	ColumnTypeOracleNVarChar2     byte = 0xcf // 207
	ColumnTypeOracleNChar         byte = 0xd0 // 208
	ColumnTypeOracleRowID         byte = 0xd1 // 209
	ColumnTypeOracleBlob          byte = 0xd2 // 210
	ColumnTypeOracleClob          byte = 0xd3 // 211
	ColumnTypeOracleCursor        byte = 0xa3 // 163
)
