// Thin, FFI-friendly wrapper around chiaki-ng's libchiaki.
// Every type here is plain C with a fixed layout so the Rust side (src/ffi.rs)
// can mirror it by hand without bindgen. Keep the two files in sync.
#ifndef PSNATIVE_H
#define PSNATIVE_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#ifdef _WIN32
#define PS_API __declspec(dllexport)
#else
#define PS_API
#endif

typedef void (*PsLogCb)(int level, const char *msg, void *user);

// Must be called once before anything else. level_mask uses chiaki's
// ChiakiLogLevel bits (1=error, 2=warning, 4=info, 8=verbose, 16=debug).
PS_API int ps_init(uint32_t level_mask, PsLogCb cb, void *user);

// ---------------------------------------------------------------- discovery

typedef struct ps_host_t
{
	int32_t state; // 0 unknown, 1 ready (awake), 2 standby (rest mode)
	int32_t is_ps5;
	uint16_t request_port;
	char host_addr[64];
	char host_name[128];
	char host_id[64];
	char system_version[32];
	char running_app_name[128];
	char running_app_titleid[32];
} PsHost;

typedef void (*PsDiscoveryCb)(const PsHost *hosts, size_t count, void *user);

typedef struct ps_discovery_t PsDiscovery;

// Broadcasts on every IPv4 interface and reports the full host list on each change.
PS_API PsDiscovery *ps_discovery_start(PsDiscoveryCb cb, void *user);
PS_API void ps_discovery_stop(PsDiscovery *d);

// Wake a console from rest mode. regist_key is the 16 byte key from registration.
PS_API int ps_wakeup(const char *host, const char regist_key[16], int ps5);

// ------------------------------------------------------------- registration

typedef struct ps_registered_host_t
{
	int32_t target;
	char server_nickname[32];
	uint8_t server_mac[6];
	char rp_regist_key[16];
	uint8_t rp_key[16];
	uint32_t rp_key_type;
	uint32_t console_pin;
} PsRegisteredHost;

// result: 0 success, 1 failed, 2 canceled. host is only valid during the call.
typedef void (*PsRegistCb)(int result, const PsRegisteredHost *host, void *user);

typedef struct ps_regist_t PsRegist;

// psn_account_id is the 8 raw bytes (base64-decode the account ID first).
// pin is the 8 digit number shown on the console's "Link Device" screen.
PS_API PsRegist *ps_regist_start(const char *host, int ps5, int broadcast,
		const uint8_t psn_account_id[8], uint32_t pin, PsRegistCb cb, void *user);
PS_API void ps_regist_free(PsRegist *r); // cancels if still running

// ------------------------------------------------------------------ session

typedef struct ps_session_config_t
{
	const char *host;
	int32_t ps5;
	char regist_key[16];
	uint8_t morning[16]; // rp_key from registration
	uint8_t psn_account_id[8];
	int32_t resolution; // 1=360p 2=540p 3=720p 4=1080p
	int32_t fps; // 30 or 60
	int32_t codec; // 0=H264 1=H265 2=H265 HDR
	uint32_t bitrate; // kbps, 0 = preset default
	int32_t hw_decode; // non-zero: try d3d11va, fall back to software
} PsSessionConfig;

enum
{
	PS_EVENT_CONNECTED = 0,
	PS_EVENT_QUIT = 1, // a = quit reason, str = description
	PS_EVENT_RUMBLE = 2, // a = left (low freq), b = right (high freq)
	PS_EVENT_LOGIN_PIN_REQUEST = 3, // a = 1 if previous pin was wrong
	PS_EVENT_NICKNAME = 4, // str = console nickname
};

typedef struct ps_session_callbacks_t
{
	void (*event)(int type, int a, int b, const char *str, void *user);
	void (*audio_settings)(uint32_t channels, uint32_t rate, void *user);
	void (*audio_frame)(const int16_t *samples, size_t frames, void *user);
	void (*frame_available)(void *user); // called on the decoder thread; just wake the UI
} PsSessionCallbacks;

typedef struct ps_session_t PsSession;

PS_API PsSession *ps_session_start(const PsSessionConfig *config, const PsSessionCallbacks *cbs, void *user);
PS_API void ps_session_stop(PsSession *s); // asynchronous, a QUIT event follows
PS_API void ps_session_free(PsSession *s); // stops, joins and frees
PS_API int ps_session_set_login_pin(PsSession *s, const char *pin);
PS_API int ps_session_go_home(PsSession *s);
PS_API int ps_session_goto_bed(PsSession *s);

typedef struct ps_controller_t
{
	uint32_t buttons; // chiaki ChiakiControllerButton bits
	uint8_t l2;
	uint8_t r2;
	int16_t left_x, left_y, right_x, right_y;
} PsController;

PS_API int ps_session_set_controller(PsSession *s, const PsController *state);

enum
{
	PS_PIXFMT_NV12 = 0, // planes: Y, interleaved UV
	PS_PIXFMT_YUV420P = 1, // planes: Y, U, V
	PS_PIXFMT_P010 = 2, // like NV12, 16 bit per sample (HDR)
	PS_PIXFMT_OTHER = 99,
};

typedef struct ps_frame_t
{
	int32_t width;
	int32_t height;
	int32_t format;
	int32_t frames_lost;
	const uint8_t *data[3];
	int32_t linesize[3];
	void *opaque; // owned AVFrame, release with ps_frame_release
} PsFrame;

// Fills *out with the newest decoded frame in system memory. Returns 1 if a
// frame was written (release it with ps_frame_release), 0 if none is ready.
PS_API int ps_session_pull_frame(PsSession *s, PsFrame *out);
PS_API void ps_frame_release(PsFrame *frame);

#ifdef __cplusplus
}
#endif

#endif
