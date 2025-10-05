#!/usr/bin/env python3
"""
Axiom SHM Test Client (Python version)

This is a Python-based Wayland client that uses shared memory (SHM) buffers
to test the Axiom compositor's rendering pipeline.

Requirements:
    pip install pywayland

Usage:
    python3 shm_test_client.py
"""

import os
import sys
import mmap
import tempfile
import struct
import signal
from pywayland.client import Display
from pywayland.protocol.wayland import (
    WlCompositor,
    WlShm,
    WlShmPool,
    WlBuffer,
    WlSurface,
)
from pywayland.protocol.xdg_shell import XdgWmBase, XdgSurface, XdgToplevel

# Window dimensions
WIDTH = 800
HEIGHT = 600
BYTES_PER_PIXEL = 4  # ARGB8888


class ShmTestClient:
    """Simple SHM-based Wayland test client"""

    def __init__(self):
        self.display = None
        self.compositor = None
        self.shm = None
        self.xdg_wm_base = None
        self.surface = None
        self.xdg_surface = None
        self.xdg_toplevel = None
        self.buffer = None
        self.shm_pool = None
        self.shm_data = None
        self.shm_fd = None
        self.configured = False
        self.running = True

    def create_shm_buffer(self, width, height):
        """Create a shared memory buffer and draw test pattern"""
        stride = width * BYTES_PER_PIXEL
        size = stride * height

        print(
            f"📐 Creating SHM buffer: {width}x{height}, stride={stride}, size={size} bytes"
        )

        # Create anonymous file
        fd = os.memfd_create("axiom-shm-test", os.MFD_CLOEXEC)
        os.ftruncate(fd, size)
        self.shm_fd = fd

        # Map the memory
        self.shm_data = mmap.mmap(
            fd, size, mmap.MAP_SHARED, mmap.PROT_READ | mmap.PROT_WRITE
        )

        # Draw test pattern
        print("🎨 Drawing test pattern...")
        self.draw_test_pattern(width, height, stride)

        # Create wl_shm_pool
        self.shm_pool = self.shm.create_pool(fd, size)

        # Create wl_buffer
        self.buffer = self.shm_pool.create_buffer(
            0,  # offset
            width,
            height,
            stride,
            WlShm.format.argb8888.value,
        )

        print(f"✅ SHM buffer created successfully")
        return self.buffer

    def draw_test_pattern(self, width, height, stride):
        """Draw a colorful test pattern to the buffer"""
        for y in range(height):
            for x in range(width):
                # Create a gradient + checkerboard pattern
                checker = ((x // 32) + (y // 32)) % 2

                if checker:
                    # Red gradient
                    r = int((x * 255) / width)
                    g = 50
                    b = 50
                else:
                    # Blue gradient
                    r = 50
                    g = int((y * 255) / height)
                    b = 200

                # ARGB8888 format (little endian)
                alpha = 0xFF
                color = (alpha << 24) | (r << 16) | (g << 8) | b

                # Write to buffer
                offset = y * stride + x * BYTES_PER_PIXEL
                struct.pack_into("<I", self.shm_data, offset, color)

        print(f"✅ Drew test pattern: {width}x{height} pixels")

    def handle_registry_global(self, registry, id, interface, version):
        """Handle registry global events"""
        print(f"📋 Registry: {interface} (id={id}, version={version})")

        if interface == "wl_compositor":
            self.compositor = registry.bind(id, WlCompositor, version)
            print("✅ Bound wl_compositor")
        elif interface == "wl_shm":
            self.shm = registry.bind(id, WlShm, version)
            print("✅ Bound wl_shm")
        elif interface == "xdg_wm_base":
            self.xdg_wm_base = registry.bind(id, XdgWmBase, version)
            self.xdg_wm_base.dispatcher["ping"] = self.handle_xdg_wm_base_ping
            print("✅ Bound xdg_wm_base")

    def handle_registry_global_remove(self, registry, id):
        """Handle registry global remove events"""
        pass

    def handle_xdg_wm_base_ping(self, xdg_wm_base, serial):
        """Handle XDG WM Base ping"""
        xdg_wm_base.pong(serial)

    def handle_xdg_surface_configure(self, xdg_surface, serial):
        """Handle XDG surface configure"""
        xdg_surface.ack_configure(serial)
        self.configured = True
        print(f"✅ XDG surface configured (serial={serial})")

        # On first configure, attach buffer and commit
        if self.buffer and self.surface and not self.configured:
            self.surface.attach(self.buffer, 0, 0)
            self.surface.damage(0, 0, WIDTH, HEIGHT)
            self.surface.commit()
            print("✅ Attached buffer and committed surface")

    def handle_xdg_toplevel_configure(self, xdg_toplevel, width, height, states):
        """Handle XDG toplevel configure"""
        if width > 0 and height > 0:
            print(f"ℹ️  Toplevel configure: {width}x{height}")

    def handle_xdg_toplevel_close(self, xdg_toplevel):
        """Handle XDG toplevel close"""
        print("🚪 Window close requested")
        self.running = False

    def setup_signal_handlers(self):
        """Setup signal handlers for clean shutdown"""

        def signal_handler(signum, frame):
            print("\n🛑 Received signal, shutting down...")
            self.running = False

        signal.signal(signal.SIGINT, signal_handler)
        signal.signal(signal.SIGTERM, signal_handler)

    def run(self):
        """Main client loop"""
        print("🚀 Starting Axiom SHM Test Client (Python)")
        print("=" * 60)
        print()

        self.setup_signal_handlers()

        # Connect to Wayland display
        try:
            self.display = Display()
            self.display.connect()
            print("✅ Connected to Wayland display")
        except Exception as e:
            print(f"❌ Failed to connect to Wayland display: {e}")
            return 1

        # Get registry and bind globals
        registry = self.display.get_registry()
        registry.dispatcher["global"] = self.handle_registry_global
        registry.dispatcher["global_remove"] = self.handle_registry_global_remove

        # Process registry
        self.display.dispatch(block=True)
        self.display.roundtrip()

        # Check that we got required interfaces
        if not self.compositor or not self.shm or not self.xdg_wm_base:
            print("❌ Missing required Wayland interfaces")
            print(f"   compositor: {self.compositor}")
            print(f"   shm: {self.shm}")
            print(f"   xdg_wm_base: {self.xdg_wm_base}")
            return 1

        print()
        print(f"📐 Creating window ({WIDTH}x{HEIGHT})")

        # Create surface
        self.surface = self.compositor.create_surface()
        print("✅ Created wl_surface")

        # Create XDG surface
        self.xdg_surface = self.xdg_wm_base.get_xdg_surface(self.surface)
        self.xdg_surface.dispatcher["configure"] = self.handle_xdg_surface_configure
        print("✅ Created xdg_surface")

        # Create XDG toplevel
        self.xdg_toplevel = self.xdg_surface.get_toplevel()
        self.xdg_toplevel.dispatcher["configure"] = self.handle_xdg_toplevel_configure
        self.xdg_toplevel.dispatcher["close"] = self.handle_xdg_toplevel_close
        self.xdg_toplevel.set_title("Axiom SHM Test (Python)")
        print("✅ Created xdg_toplevel")

        # Create SHM buffer
        print()
        print("🎨 Creating SHM buffer")
        try:
            self.create_shm_buffer(WIDTH, HEIGHT)
        except Exception as e:
            print(f"❌ Failed to create SHM buffer: {e}")
            import traceback

            traceback.print_exc()
            return 1

        # Initial commit to map the window
        self.surface.commit()
        print("✅ Committed initial surface")

        # Wait for configure
        print()
        print("⏳ Waiting for configure event...")
        while not self.configured and self.running:
            self.display.dispatch(block=True)

        if self.configured:
            # Attach buffer after configure
            self.surface.attach(self.buffer, 0, 0)
            self.surface.damage(0, 0, WIDTH, HEIGHT)
            self.surface.commit()

            print()
            print("✨ Window is now visible and should display test pattern!")
            print("   - Red/blue checkerboard with gradients")
            print("   - Press Ctrl+C to exit")
            print()

        # Main event loop
        print("🔄 Entering main loop...")
        while self.running:
            try:
                self.display.dispatch(block=True)
            except Exception as e:
                print(f"❌ Display dispatch error: {e}")
                break

        # Cleanup
        print()
        print("🧹 Cleaning up...")
        self.cleanup()

        print("✅ Shutdown complete")
        return 0

    def cleanup(self):
        """Clean up resources"""
        if self.buffer:
            self.buffer.destroy()
        if self.shm_pool:
            self.shm_pool.destroy()
        if self.shm_data:
            self.shm_data.close()
        if self.shm_fd:
            os.close(self.shm_fd)
        if self.xdg_toplevel:
            self.xdg_toplevel.destroy()
        if self.xdg_surface:
            self.xdg_surface.destroy()
        if self.surface:
            self.surface.destroy()
        if self.xdg_wm_base:
            self.xdg_wm_base.destroy()
        if self.compositor:
            self.compositor.destroy()
        if self.shm:
            self.shm.destroy()
        if self.display:
            self.display.disconnect()


def main():
    """Main entry point"""
    # Check for pywayland
    try:
        import pywayland

        print(f"ℹ️  Using pywayland version: {pywayland.__version__}")
    except ImportError:
        print("❌ pywayland not found")
        print("   Install with: pip install pywayland")
        return 1

    # Check for memfd_create (Python 3.8+)
    if not hasattr(os, "memfd_create"):
        print("❌ os.memfd_create not available (Python 3.8+ required)")
        return 1

    # Run the client
    client = ShmTestClient()
    try:
        return client.run()
    except KeyboardInterrupt:
        print("\n🛑 Interrupted by user")
        client.cleanup()
        return 0
    except Exception as e:
        print(f"❌ Unexpected error: {e}")
        import traceback

        traceback.print_exc()
        client.cleanup()
        return 1


if __name__ == "__main__":
    sys.exit(main())
