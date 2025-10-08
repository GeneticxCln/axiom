#!/usr/bin/env python3
"""
Simple test script to verify Axiom IPC communication.

This script demonstrates how Lazy UI can communicate with the Axiom compositor
via Unix sockets using the JSON message protocol.
"""

import json
import socket
import time
import sys

def test_axiom_ipc():
    """Test communication with Axiom compositor via IPC."""
    import os
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
    socket_path = os.path.join(runtime_dir, "axiom", "axiom.sock")
    
    print("🔍 Testing Axiom IPC communication...")
    
    try:
        # Connect to Axiom's IPC socket
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(5.0)
        sock.connect(socket_path)
        print(f"✅ Connected to Axiom IPC socket: {socket_path}")
        
        # Wait for startup message from Axiom
        response = sock.recv(4096).decode().strip()
        if response:
            startup_data = json.loads(response)
            print(f"📨 Received startup notification: {startup_data}")
        
        # Test 1: Send health check request
        print("\n🏥 Testing health check...")
        health_check = {"type": "HealthCheck"}
        sock.send((json.dumps(health_check) + "\\n").encode())
        
        response = sock.recv(4096).decode().strip()
        if response:
            health_data = json.loads(response)
            print(f"📊 Health check response: {health_data}")
        
        # Test 2: Send configuration query
        print("\n⚙️ Testing configuration query...")
        config_query = {"type": "GetConfig", "key": "workspace.count"}
        sock.send((json.dumps(config_query) + "\\n").encode())
        
        response = sock.recv(4096).decode().strip()
        if response:
            config_data = json.loads(response)
            print(f"📋 Config response: {config_data}")
        
        # Test 3: Send optimization command
        print("\n🎯 Testing AI optimization...")
        optimization = {
            "type": "OptimizeConfig",
            "changes": {
                "effects.blur_radius": 5.0,
                "workspace.animation_speed": 0.8
            },
            "reason": "AI performance optimization based on usage patterns"
        }
        sock.send((json.dumps(optimization) + "\\n").encode())
        
        print("✅ All IPC tests completed successfully!")
        
        sock.close()
        
    except FileNotFoundError:
        print(f"❌ Axiom IPC socket not found: {socket_path}")
        print("   Make sure Axiom compositor is running")
        return False
        
    except ConnectionRefusedError:
        print(f"❌ Connection refused to Axiom IPC socket")
        print("   Make sure Axiom compositor is running")
        return False
        
    except Exception as e:
        print(f"❌ IPC test failed: {e}")
        return False
    
    return True

def main():
    """Main test function."""
    print("🚀 Axiom IPC Communication Test")
    print("=" * 40)
    
    # Test if socket exists
    import os
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
    socket_path = os.path.join(runtime_dir, "axiom", "axiom.sock")
    
    if not os.path.exists(socket_path):
        print(f"⚠️  Socket file not found: {socket_path}")
        print("   To test IPC communication:")
        print("   1. Start Axiom in one terminal: ./target/debug/axiom --debug --windowed")
        print("   2. Run this test in another terminal: python3 test_ipc.py")
        return 1
    
    success = test_axiom_ipc()
    return 0 if success else 1

if __name__ == "__main__":
    sys.exit(main())
