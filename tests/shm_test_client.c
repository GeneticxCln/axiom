/*
 * Minimal SHM-based Wayland Test Client
 * 
 * This program creates a simple window and renders content using shared memory.
 * Used to test the Axiom compositor's rendering pipeline.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <errno.h>
#include <wayland-client.h>
#include <wayland-client-protocol.h>

/* XDG Shell protocol - you'll need to generate these from the XML */
#include "xdg-shell-client-protocol.h"

#define WIDTH 800
#define HEIGHT 600

struct client_state {
    struct wl_display *display;
    struct wl_registry *registry;
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct xdg_wm_base *xdg_wm_base;
    struct wl_surface *surface;
    struct xdg_surface *xdg_surface;
    struct xdg_toplevel *xdg_toplevel;
    struct wl_buffer *buffer;
    struct wl_shm_pool *pool;
    void *shm_data;
    int running;
    int configured;
};

/* Create anonymous file for shared memory */
static int create_shm_file(size_t size) {
    char name[256];
    int fd;
    
    snprintf(name, sizeof(name), "/axiom-shm-test-%d", getpid());
    
    fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    if (fd < 0) {
        fprintf(stderr, "shm_open failed: %s\n", strerror(errno));
        return -1;
    }
    
    shm_unlink(name);
    
    if (ftruncate(fd, size) < 0) {
        fprintf(stderr, "ftruncate failed: %s\n", strerror(errno));
        close(fd);
        return -1;
    }
    
    return fd;
}

/* Draw a test pattern to the buffer */
static void draw_test_pattern(uint32_t *pixels, int width, int height) {
    for (int y = 0; y < height; y++) {
        for (int x = 0; x < width; x++) {
            uint32_t color;
            
            /* Create a gradient + checkerboard pattern */
            int checker = ((x / 32) + (y / 32)) % 2;
            
            if (checker) {
                /* Red gradient */
                uint8_t r = (x * 255) / width;
                uint8_t g = 50;
                uint8_t b = 50;
                color = 0xFF000000 | (r << 16) | (g << 8) | b;
            } else {
                /* Blue gradient */
                uint8_t r = 50;
                uint8_t g = (y * 255) / height;
                uint8_t b = 200;
                color = 0xFF000000 | (r << 16) | (g << 8) | b;
            }
            
            pixels[y * width + x] = color;
        }
    }
    
    printf("✅ Drew test pattern: %dx%d pixels\n", width, height);
}

/* Create shared memory buffer */
static struct wl_buffer *create_buffer(struct client_state *state, int width, int height) {
    int stride = width * 4; /* 4 bytes per pixel (ARGB8888) */
    int size = stride * height;
    
    /* Create shared memory file */
    int fd = create_shm_file(size);
    if (fd < 0) {
        return NULL;
    }
    
    /* Map the memory */
    void *data = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (data == MAP_FAILED) {
        fprintf(stderr, "mmap failed: %s\n", strerror(errno));
        close(fd);
        return NULL;
    }
    
    state->shm_data = data;
    
    /* Draw the test pattern */
    draw_test_pattern((uint32_t *)data, width, height);
    
    /* Create wl_shm_pool */
    state->pool = wl_shm_create_pool(state->shm, fd, size);
    close(fd);
    
    if (!state->pool) {
        fprintf(stderr, "Failed to create wl_shm_pool\n");
        munmap(data, size);
        return NULL;
    }
    
    /* Create wl_buffer from pool */
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(
        state->pool, 0, width, height, stride, WL_SHM_FORMAT_ARGB8888
    );
    
    if (!buffer) {
        fprintf(stderr, "Failed to create wl_buffer\n");
        wl_shm_pool_destroy(state->pool);
        munmap(data, size);
        return NULL;
    }
    
    printf("✅ Created SHM buffer: %dx%d, stride=%d, size=%d bytes\n", 
           width, height, stride, size);
    
    return buffer;
}

/* XDG Surface configure handler */
static void xdg_surface_configure(void *data, struct xdg_surface *xdg_surface, uint32_t serial) {
    struct client_state *state = data;
    xdg_surface_ack_configure(xdg_surface, serial);
    
    state->configured = 1;
    printf("✅ XDG surface configured (serial=%u)\n", serial);
    
    /* On first configure, attach buffer and commit */
    if (state->buffer && state->surface) {
        wl_surface_attach(state->surface, state->buffer, 0, 0);
        wl_surface_damage(state->surface, 0, 0, WIDTH, HEIGHT);
        wl_surface_commit(state->surface);
        printf("✅ Attached buffer and committed surface\n");
    }
}

static const struct xdg_surface_listener xdg_surface_listener = {
    .configure = xdg_surface_configure,
};

/* XDG Toplevel configure handler */
static void xdg_toplevel_configure(void *data, struct xdg_toplevel *xdg_toplevel,
                                   int32_t width, int32_t height, struct wl_array *states) {
    printf("ℹ️  Toplevel configure: %dx%d\n", width, height);
}

static void xdg_toplevel_close(void *data, struct xdg_toplevel *xdg_toplevel) {
    struct client_state *state = data;
    state->running = 0;
    printf("🚪 Window close requested\n");
}

static const struct xdg_toplevel_listener xdg_toplevel_listener = {
    .configure = xdg_toplevel_configure,
    .close = xdg_toplevel_close,
};

/* XDG WM Base ping handler */
static void xdg_wm_base_ping(void *data, struct xdg_wm_base *xdg_wm_base, uint32_t serial) {
    xdg_wm_base_pong(xdg_wm_base, serial);
}

static const struct xdg_wm_base_listener xdg_wm_base_listener = {
    .ping = xdg_wm_base_ping,
};

/* Registry global handler */
static void registry_global(void *data, struct wl_registry *registry,
                           uint32_t name, const char *interface, uint32_t version) {
    struct client_state *state = data;
    
    printf("📋 Registry: %s (id=%u, version=%u)\n", interface, name, version);
    
    if (strcmp(interface, "wl_compositor") == 0) {
        state->compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
        printf("✅ Bound wl_compositor\n");
    } else if (strcmp(interface, "wl_shm") == 0) {
        state->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
        printf("✅ Bound wl_shm\n");
    } else if (strcmp(interface, "xdg_wm_base") == 0) {
        state->xdg_wm_base = wl_registry_bind(registry, name, &xdg_wm_base_interface, 1);
        xdg_wm_base_add_listener(state->xdg_wm_base, &xdg_wm_base_listener, state);
        printf("✅ Bound xdg_wm_base\n");
    }
}

static void registry_global_remove(void *data, struct wl_registry *registry, uint32_t name) {
    /* Not used */
}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_global_remove,
};

int main(int argc, char *argv[]) {
    struct client_state state = {0};
    state.running = 1;
    
    printf("🚀 Starting Axiom SHM Test Client\n");
    printf("================================\n\n");
    
    /* Connect to Wayland display */
    state.display = wl_display_connect(NULL);
    if (!state.display) {
        fprintf(stderr, "❌ Failed to connect to Wayland display\n");
        return 1;
    }
    printf("✅ Connected to Wayland display\n");
    
    /* Get registry and bind globals */
    state.registry = wl_display_get_registry(state.display);
    wl_registry_add_listener(state.registry, &registry_listener, &state);
    wl_display_roundtrip(state.display);
    
    /* Check that we got all required interfaces */
    if (!state.compositor || !state.shm || !state.xdg_wm_base) {
        fprintf(stderr, "❌ Missing required Wayland interfaces\n");
        fprintf(stderr, "   compositor: %p, shm: %p, xdg_wm_base: %p\n",
                state.compositor, state.shm, state.xdg_wm_base);
        return 1;
    }
    
    printf("\n📐 Creating window (%dx%d)\n", WIDTH, HEIGHT);
    
    /* Create surface */
    state.surface = wl_compositor_create_surface(state.compositor);
    if (!state.surface) {
        fprintf(stderr, "❌ Failed to create wl_surface\n");
        return 1;
    }
    printf("✅ Created wl_surface\n");
    
    /* Create XDG surface */
    state.xdg_surface = xdg_wm_base_get_xdg_surface(state.xdg_wm_base, state.surface);
    if (!state.xdg_surface) {
        fprintf(stderr, "❌ Failed to create xdg_surface\n");
        return 1;
    }
    xdg_surface_add_listener(state.xdg_surface, &xdg_surface_listener, &state);
    printf("✅ Created xdg_surface\n");
    
    /* Create XDG toplevel */
    state.xdg_toplevel = xdg_surface_get_toplevel(state.xdg_surface);
    if (!state.xdg_toplevel) {
        fprintf(stderr, "❌ Failed to create xdg_toplevel\n");
        return 1;
    }
    xdg_toplevel_add_listener(state.xdg_toplevel, &xdg_toplevel_listener, &state);
    xdg_toplevel_set_title(state.xdg_toplevel, "Axiom SHM Test");
    printf("✅ Created xdg_toplevel\n");
    
    /* Create shared memory buffer */
    printf("\n🎨 Creating SHM buffer\n");
    state.buffer = create_buffer(&state, WIDTH, HEIGHT);
    if (!state.buffer) {
        fprintf(stderr, "❌ Failed to create buffer\n");
        return 1;
    }
    
    /* Initial commit to map the window */
    wl_surface_commit(state.surface);
    printf("✅ Committed initial surface\n");
    
    /* Wait for configure */
    printf("\n⏳ Waiting for configure event...\n");
    while (!state.configured && state.running) {
        if (wl_display_dispatch(state.display) == -1) {
            fprintf(stderr, "❌ Display dispatch failed\n");
            break;
        }
    }
    
    if (state.configured) {
        printf("\n✨ Window is now visible and should display test pattern!\n");
        printf("   - Red/blue checkerboard with gradients\n");
        printf("   - Press Ctrl+C to exit\n\n");
    }
    
    /* Main event loop */
    printf("🔄 Entering main loop...\n");
    while (state.running) {
        if (wl_display_dispatch(state.display) == -1) {
            fprintf(stderr, "❌ Display dispatch failed\n");
            break;
        }
    }
    
    /* Cleanup */
    printf("\n🧹 Cleaning up...\n");
    
    if (state.buffer) wl_buffer_destroy(state.buffer);
    if (state.pool) wl_shm_pool_destroy(state.pool);
    if (state.shm_data) munmap(state.shm_data, WIDTH * HEIGHT * 4);
    if (state.xdg_toplevel) xdg_toplevel_destroy(state.xdg_toplevel);
    if (state.xdg_surface) xdg_surface_destroy(state.xdg_surface);
    if (state.surface) wl_surface_destroy(state.surface);
    if (state.xdg_wm_base) xdg_wm_base_destroy(state.xdg_wm_base);
    if (state.compositor) wl_compositor_destroy(state.compositor);
    if (state.shm) wl_shm_destroy(state.shm);
    if (state.registry) wl_registry_destroy(state.registry);
    if (state.display) wl_display_disconnect(state.display);
    
    printf("✅ Shutdown complete\n");
    return 0;
}