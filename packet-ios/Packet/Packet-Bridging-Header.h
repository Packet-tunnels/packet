#ifndef Packet_Bridging_Header_h
#define Packet_Bridging_Header_h

#include <stdint.h>

// Set log callback to intercept Rust logs
void phantom_set_log_callback(void (*cb)(const char*));
void phantom_emit_test_output(void);
char *phantom_copy_stats_json(void);
char *phantom_copy_mesh_stats_json(void);
void phantom_free_string(char *value);
void phantom_stop_client(void);
void phantom_stop_layered_carrier(void);

// DirectSock start: local mixed HTTP/SOCKS proxy -> Trojan TCP/WS TLS carrier
int32_t phantom_start_layered_carrier(
    const char *trojan_uri,
    uint16_t listen_port
);
int32_t phantom_start_layered_carrier_full(
    const char *trojan_uri,
    uint16_t listen_port,
    int32_t fragment_enabled,
    uint32_t fragment_size
);

// Basic start
int32_t phantom_start(
    const char *server_url,
    const char *secret,
    uint16_t listen_port
);

// CDN bypass start (for Iran)
int32_t phantom_start_cdn(
    const char *server_url,
    const char *secret,
    uint16_t listen_port,
    const char *cdn_edge,
    const char *host_override,
    int32_t transport_mode
);

// Full bypass start: CDN edge + Host + SNI + TLS fragmentation + TLS profile
int32_t phantom_start_full(
    const char *server_url,
    const char *secret,
    uint16_t listen_port,
    const char *cdn_edge,
    const char *host_override,
    const char *sni_override,
    int32_t transport_mode,
    int32_t fragment_enabled,
    uint32_t fragment_size,
    int32_t tls_profile,
    const char *obfs_key
);

// Native Packet mesh start
int32_t phantom_start_mesh(
    const char *config_json,
    uint16_t listen_port
);

// Import Packet peer descriptors
int32_t phantom_import_mesh_peers(const char *peers_json);

#endif /* Packet_Bridging_Header_h */
