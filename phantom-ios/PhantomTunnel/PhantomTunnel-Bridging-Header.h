#ifndef PhantomTunnel_Bridging_Header_h
#define PhantomTunnel_Bridging_Header_h

#include <stdint.h>

// Set log callback to intercept Rust logs
void phantom_set_log_callback(void (*cb)(const char*));
void phantom_emit_test_output(void);

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

#endif /* PhantomTunnel_Bridging_Header_h */
