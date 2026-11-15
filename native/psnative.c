// See psnative.h. This file is the only place that touches libchiaki's structs.

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#include <iphlpapi.h>
#endif

#include "psnative.h"

#include <chiaki/common.h>
#include <chiaki/log.h>
#include <chiaki/discovery.h>
#include <chiaki/discoveryservice.h>
#include <chiaki/regist.h>
#include <chiaki/session.h>
#include <chiaki/controller.h>
#include <chiaki/ffmpegdecoder.h>
#include <chiaki/opusdecoder.h>

#include <libavutil/frame.h>
#include <libavutil/hwcontext.h>
#include <libavutil/pixfmt.h>

#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

static ChiakiLog g_log;
static PsLogCb g_log_cb;
static void *g_log_user;

static void log_cb(ChiakiLogLevel level, const char *msg, void *user)
{
	(void)user;
	if(g_log_cb)
		g_log_cb((int)level, msg, g_log_user);
}

static void copy_str(char *dst, size_t dst_size, const char *src)
{
	if(!dst_size)
		return;
	if(!src)
	{
		dst[0] = '\0';
		return;
	}
	size_t n = strlen(src);
	if(n >= dst_size)
		n = dst_size - 1;
	memcpy(dst, src, n);
	dst[n] = '\0';
}

PS_API int ps_init(uint32_t level_mask, PsLogCb cb, void *user)
{
	g_log_cb = cb;
	g_log_user = user;
	chiaki_log_init(&g_log, level_mask, log_cb, NULL);
	return (int)chiaki_lib_init();
}

// ---------------------------------------------------------------- discovery

struct ps_discovery_t
{
	ChiakiDiscoveryService service;
	PsDiscoveryCb cb;
	void *user;
};

static void discovery_cb(ChiakiDiscoveryHost *hosts, size_t count, void *user)
{
	PsDiscovery *d = user;
	PsHost *out = calloc(count ? count : 1, sizeof(PsHost));
	if(!out)
		return;
	for(size_t i = 0; i < count; i++)
	{
		ChiakiDiscoveryHost *h = &hosts[i];
		PsHost *o = &out[i];
		o->state = (int32_t)h->state;
		o->is_ps5 = chiaki_discovery_host_is_ps5(h) ? 1 : 0;
		o->request_port = h->host_request_port;
		copy_str(o->host_addr, sizeof(o->host_addr), h->host_addr);
		copy_str(o->host_name, sizeof(o->host_name), h->host_name);
		copy_str(o->host_id, sizeof(o->host_id), h->host_id);
		copy_str(o->system_version, sizeof(o->system_version), h->system_version);
		copy_str(o->running_app_name, sizeof(o->running_app_name), h->running_app_name);
		copy_str(o->running_app_titleid, sizeof(o->running_app_titleid), h->running_app_titleid);
	}
	d->cb(out, count, d->user);
	free(out);
}

// Directed broadcast address of every active IPv4 adapter, so discovery works
// even when 255.255.255.255 only leaves through one interface.
static size_t collect_broadcast_addrs(struct sockaddr_storage **out)
{
	*out = NULL;
#ifdef _WIN32
	ULONG len = 0;
	if(GetAdaptersInfo(NULL, &len) != ERROR_BUFFER_OVERFLOW)
		return 0;
	IP_ADAPTER_INFO *info = malloc(len);
	if(!info)
		return 0;
	if(GetAdaptersInfo(info, &len) != NO_ERROR)
	{
		free(info);
		return 0;
	}
	size_t cap = 16, n = 0;
	struct sockaddr_storage *addrs = calloc(cap, sizeof(*addrs));
	for(IP_ADAPTER_INFO *a = info; a && addrs; a = a->Next)
	{
		for(IP_ADDR_STRING *ip = &a->IpAddressList; ip; ip = ip->Next)
		{
			struct in_addr addr, mask;
			if(inet_pton(AF_INET, ip->IpAddress.String, &addr) != 1
					|| inet_pton(AF_INET, ip->IpMask.String, &mask) != 1
					|| addr.s_addr == 0)
				continue;
			uint32_t bcast = addr.s_addr | ~mask.s_addr;
			bool dup = false;
			for(size_t i = 0; i < n; i++)
				if(((struct sockaddr_in *)&addrs[i])->sin_addr.s_addr == bcast)
					dup = true;
			if(dup || n == cap)
				continue;
			struct sockaddr_in *sin = (struct sockaddr_in *)&addrs[n++];
			sin->sin_family = AF_INET;
			sin->sin_addr.s_addr = bcast;
		}
	}
	free(info);
	if(!n)
	{
		free(addrs);
		return 0;
	}
	*out = addrs;
	return n;
#else
	return 0;
#endif
}

PS_API PsDiscovery *ps_discovery_start(PsDiscoveryCb cb, void *user)
{
	PsDiscovery *d = calloc(1, sizeof(PsDiscovery));
	if(!d)
		return NULL;
	d->cb = cb;
	d->user = user;

	ChiakiDiscoveryServiceOptions options = {0};
	options.ping_ms = 500;
	options.ping_initial_ms = 500;
	options.hosts_max = 16;
	options.host_drop_pings = 3;
	options.cb = discovery_cb;
	options.cb_user = d;

	struct sockaddr_in in_addr = {0};
	in_addr.sin_family = AF_INET;
	in_addr.sin_addr.s_addr = 0xffffffff;
	struct sockaddr_storage addr = {0};
	memcpy(&addr, &in_addr, sizeof(in_addr));
	options.send_addr = &addr;
	options.send_addr_size = sizeof(in_addr);
	options.broadcast_num = collect_broadcast_addrs(&options.broadcast_addrs);

	ChiakiErrorCode err = chiaki_discovery_service_init(&d->service, &options, &g_log);
	free(options.broadcast_addrs);
	if(err != CHIAKI_ERR_SUCCESS)
	{
		CHIAKI_LOGE(&g_log, "Discovery failed to start: %s", chiaki_error_string(err));
		free(d);
		return NULL;
	}
	return d;
}

PS_API void ps_discovery_stop(PsDiscovery *d)
{
	if(!d)
		return;
	chiaki_discovery_service_fini(&d->service);
	free(d);
}

PS_API int ps_wakeup(const char *host, const char regist_key[16], int ps5)
{
	// The wakeup credential is the regist key read as a hex number.
	char key[17] = {0};
	memcpy(key, regist_key, 16);
	size_t len = strlen(key);
	if(len == 0 || len > 8)
		return CHIAKI_ERR_INVALID_DATA;
	char *end = NULL;
	uint64_t credential = strtoull(key, &end, 16);
	if(!end || *end != '\0')
		return CHIAKI_ERR_INVALID_DATA;
	return (int)chiaki_discovery_wakeup(&g_log, NULL, host, credential, ps5 != 0);
}

// ------------------------------------------------------------- registration

struct ps_regist_t
{
	ChiakiRegist regist;
	PsRegistCb cb;
	void *user;
};

static void regist_cb(ChiakiRegistEvent *event, void *user)
{
	PsRegist *r = user;
	switch(event->type)
	{
		case CHIAKI_REGIST_EVENT_TYPE_FINISHED_SUCCESS:
		{
			ChiakiRegisteredHost *h = event->registered_host;
			PsRegisteredHost out = {0};
			out.target = (int32_t)h->target;
			memcpy(out.server_nickname, h->server_nickname, sizeof(out.server_nickname));
			out.server_nickname[sizeof(out.server_nickname) - 1] = '\0';
			memcpy(out.server_mac, h->server_mac, sizeof(out.server_mac));
			memcpy(out.rp_regist_key, h->rp_regist_key, sizeof(out.rp_regist_key));
			memcpy(out.rp_key, h->rp_key, sizeof(out.rp_key));
			out.rp_key_type = h->rp_key_type;
			out.console_pin = h->console_pin;
			r->cb(0, &out, r->user);
			break;
		}
		case CHIAKI_REGIST_EVENT_TYPE_FINISHED_CANCELED:
			r->cb(2, NULL, r->user);
			break;
		default:
			r->cb(1, NULL, r->user);
			break;
	}
}

PS_API PsRegist *ps_regist_start(const char *host, int ps5, int broadcast,
		const uint8_t psn_account_id[8], uint32_t pin, PsRegistCb cb, void *user)
{
	PsRegist *r = calloc(1, sizeof(PsRegist));
	if(!r)
		return NULL;
	r->cb = cb;
	r->user = user;

	ChiakiRegistInfo info = {0};
	info.target = ps5 ? CHIAKI_TARGET_PS5_1 : CHIAKI_TARGET_PS4_10;
	info.host = host;
	info.broadcast = broadcast != 0;
	info.psn_online_id = NULL;
	memcpy(info.psn_account_id, psn_account_id, CHIAKI_PSN_ACCOUNT_ID_SIZE);
	info.pin = pin;
	info.console_pin = 0;
	info.holepunch_info = NULL;
	info.rudp = NULL;

	ChiakiErrorCode err = chiaki_regist_start(&r->regist, &g_log, &info, regist_cb, r);
	if(err != CHIAKI_ERR_SUCCESS)
	{
		CHIAKI_LOGE(&g_log, "Registration failed to start: %s", chiaki_error_string(err));
		free(r);
		return NULL;
	}
	return r;
}

PS_API void ps_regist_free(PsRegist *r)
{
	if(!r)
		return;
	chiaki_regist_stop(&r->regist);
	chiaki_regist_fini(&r->regist);
	free(r);
}

// ------------------------------------------------------------------ session

struct ps_session_t
{
	ChiakiSession session;
	ChiakiFfmpegDecoder decoder;
	ChiakiOpusDecoder opus;
	PsSessionCallbacks cbs;
	void *user;
	char *host;
	ChiakiControllerState controller;
};

static void session_event_cb(ChiakiEvent *event, void *user)
{
	PsSession *s = user;
	if(!s->cbs.event)
		return;
	switch(event->type)
	{
		case CHIAKI_EVENT_CONNECTED:
			s->cbs.event(PS_EVENT_CONNECTED, 0, 0, NULL, s->user);
			break;
		case CHIAKI_EVENT_QUIT:
			s->cbs.event(PS_EVENT_QUIT, (int)event->quit.reason, 0,
					event->quit.reason_str ? event->quit.reason_str : chiaki_quit_reason_string(event->quit.reason),
					s->user);
			break;
		case CHIAKI_EVENT_RUMBLE:
			s->cbs.event(PS_EVENT_RUMBLE, event->rumble.left, event->rumble.right, NULL, s->user);
			break;
		case CHIAKI_EVENT_LOGIN_PIN_REQUEST:
			s->cbs.event(PS_EVENT_LOGIN_PIN_REQUEST, event->login_pin_request.pin_incorrect ? 1 : 0, 0, NULL, s->user);
			break;
		case CHIAKI_EVENT_NICKNAME_RECEIVED:
		{
			char nick[sizeof(event->server_nickname) + 1] = {0};
			memcpy(nick, event->server_nickname, sizeof(event->server_nickname));
			s->cbs.event(PS_EVENT_NICKNAME, 0, 0, nick, s->user);
			break;
		}
		default:
			break;
	}
}

static void frame_available_cb(ChiakiFfmpegDecoder *decoder, void *user)
{
	(void)decoder;
	PsSession *s = user;
	if(s->cbs.frame_available)
		s->cbs.frame_available(s->user);
}

static void audio_settings_cb(uint32_t channels, uint32_t rate, void *user)
{
	PsSession *s = user;
	if(s->cbs.audio_settings)
		s->cbs.audio_settings(channels, rate, s->user);
}

static void audio_frame_cb(int16_t *buf, size_t samples_count, void *user)
{
	PsSession *s = user;
	if(s->cbs.audio_frame)
		s->cbs.audio_frame(buf, samples_count, s->user);
}

PS_API PsSession *ps_session_start(const PsSessionConfig *config, const PsSessionCallbacks *cbs, void *user)
{
	PsSession *s = calloc(1, sizeof(PsSession));
	if(!s)
		return NULL;
	s->cbs = *cbs;
	s->user = user;
	s->host = strdup(config->host);
	chiaki_controller_state_set_idle(&s->controller);

	ChiakiCodec codec = config->ps5 ? (ChiakiCodec)config->codec : CHIAKI_CODEC_H264;
	unsigned int fps = config->fps == 30 ? 30 : 60;

	ChiakiErrorCode err = CHIAKI_ERR_UNKNOWN;
	if(config->hw_decode)
	{
		err = chiaki_ffmpeg_decoder_init(&s->decoder, &g_log, codec, fps, "d3d11va", NULL, frame_available_cb, s);
		if(err != CHIAKI_ERR_SUCCESS)
			CHIAKI_LOGW(&g_log, "Hardware decoding unavailable, using software decoding");
	}
	if(err != CHIAKI_ERR_SUCCESS)
		err = chiaki_ffmpeg_decoder_init(&s->decoder, &g_log, codec, fps, NULL, NULL, frame_available_cb, s);
	if(err != CHIAKI_ERR_SUCCESS)
	{
		CHIAKI_LOGE(&g_log, "Video decoder init failed");
		goto error_host;
	}

	chiaki_opus_decoder_init(&s->opus, &g_log);

	ChiakiConnectInfo info;
	memset(&info, 0, sizeof(info));
	info.ps5 = config->ps5 != 0;
	info.host = s->host;
	memcpy(info.regist_key, config->regist_key, sizeof(info.regist_key));
	memcpy(info.morning, config->morning, sizeof(info.morning));
	chiaki_connect_video_profile_preset(&info.video_profile,
			(ChiakiVideoResolutionPreset)config->resolution, (ChiakiVideoFPSPreset)fps);
	info.video_profile.codec = codec;
	if(config->bitrate)
		info.video_profile.bitrate = config->bitrate;
	info.video_profile_auto_downgrade = true;
	info.enable_keyboard = false;
	info.enable_dualsense = config->ps5 != 0;
	info.auto_regist = false;
	info.holepunch_session = NULL;
	info.rudp_sock = NULL;
	memcpy(info.psn_account_id, config->psn_account_id, sizeof(info.psn_account_id));
	info.packet_loss_max = 0.05;
	info.enable_idr_on_fec_failure = true;

	err = chiaki_session_init(&s->session, &info, &g_log);
	if(err != CHIAKI_ERR_SUCCESS)
	{
		CHIAKI_LOGE(&g_log, "Session init failed: %s", chiaki_error_string(err));
		goto error_decoders;
	}

	chiaki_opus_decoder_set_cb(&s->opus, audio_settings_cb, audio_frame_cb, s);
	ChiakiAudioSink audio_sink;
	chiaki_opus_decoder_get_sink(&s->opus, &audio_sink);
	chiaki_session_set_audio_sink(&s->session, &audio_sink);
	chiaki_session_set_video_sample_cb(&s->session, chiaki_ffmpeg_decoder_video_sample_cb, &s->decoder);
	chiaki_session_set_event_cb(&s->session, session_event_cb, s);

	err = chiaki_session_start(&s->session);
	if(err != CHIAKI_ERR_SUCCESS)
	{
		CHIAKI_LOGE(&g_log, "Session start failed: %s", chiaki_error_string(err));
		chiaki_session_fini(&s->session);
		goto error_decoders;
	}
	return s;

error_decoders:
	chiaki_opus_decoder_fini(&s->opus);
	chiaki_ffmpeg_decoder_fini(&s->decoder);
error_host:
	free(s->host);
	free(s);
	return NULL;
}

PS_API void ps_session_stop(PsSession *s)
{
	if(s)
		chiaki_session_stop(&s->session);
}

PS_API void ps_session_free(PsSession *s)
{
	if(!s)
		return;
	chiaki_session_stop(&s->session);
	chiaki_session_join(&s->session);
	chiaki_session_fini(&s->session);
	chiaki_opus_decoder_fini(&s->opus);
	chiaki_ffmpeg_decoder_fini(&s->decoder);
	free(s->host);
	free(s);
}

PS_API int ps_session_set_login_pin(PsSession *s, const char *pin)
{
	return (int)chiaki_session_set_login_pin(&s->session, (const uint8_t *)pin, strlen(pin));
}

PS_API int ps_session_go_home(PsSession *s)
{
	return (int)chiaki_session_go_home(&s->session);
}

PS_API int ps_session_goto_bed(PsSession *s)
{
	return (int)chiaki_session_goto_bed(&s->session);
}

PS_API int ps_session_set_controller(PsSession *s, const PsController *state)
{
	s->controller.buttons = state->buttons;
	s->controller.l2_state = state->l2;
	s->controller.r2_state = state->r2;
	s->controller.left_x = state->left_x;
	s->controller.left_y = state->left_y;
	s->controller.right_x = state->right_x;
	s->controller.right_y = state->right_y;
	return (int)chiaki_session_set_controller_state(&s->session, &s->controller);
}

static int32_t map_pix_fmt(int fmt)
{
	switch(fmt)
	{
		case AV_PIX_FMT_NV12: return PS_PIXFMT_NV12;
		case AV_PIX_FMT_YUV420P:
		case AV_PIX_FMT_YUVJ420P: return PS_PIXFMT_YUV420P;
		case AV_PIX_FMT_P010LE: return PS_PIXFMT_P010;
		default: return PS_PIXFMT_OTHER;
	}
}

PS_API int ps_session_pull_frame(PsSession *s, PsFrame *out)
{
	int32_t frames_lost = 0;
	ChiakiFfmpegFrame f = chiaki_ffmpeg_decoder_pull_frame(&s->decoder, &frames_lost);
	AVFrame *frame = f.frame;
	if(!frame)
		return 0;

	if(frame->hw_frames_ctx)
	{
		// Copy GPU surface to system memory (NV12 / P010).
		AVFrame *sw = av_frame_alloc();
		if(!sw || av_hwframe_transfer_data(sw, frame, 0) < 0)
		{
			av_frame_free(&sw);
			av_frame_free(&frame);
			return 0;
		}
		av_frame_free(&frame);
		frame = sw;
	}

	memset(out, 0, sizeof(*out));
	out->width = frame->width;
	out->height = frame->height;
	out->format = map_pix_fmt(frame->format);
	out->frames_lost = frames_lost;
	for(int i = 0; i < 3; i++)
	{
		out->data[i] = frame->data[i];
		out->linesize[i] = frame->linesize[i];
	}
	out->opaque = frame;
	return 1;
}

PS_API void ps_frame_release(PsFrame *frame)
{
	if(!frame || !frame->opaque)
		return;
	AVFrame *f = frame->opaque;
	av_frame_free(&f);
	frame->opaque = NULL;
}
