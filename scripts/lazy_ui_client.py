#!/usr/bin/env python3
"""
Lazy UI - AI-powered Wayland Compositor Optimization System

This is the Python client that connects to the Axiom compositor via IPC
to provide real-time AI-driven optimization based on user behavior and
system performance metrics.

Key Features:
- Real-time performance monitoring and analysis
- User behavior pattern recognition
- AI-driven configuration optimization
- Predictive performance tuning
- Adaptive visual effects management
"""

import asyncio
import json
import socket
import time
import sys
import logging
import statistics
from dataclasses import dataclass
from typing import Dict, List, Optional, Any
from collections import deque, defaultdict
import signal

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger('LazyUI')

@dataclass
class PerformanceMetrics:
    """Performance metrics from the compositor"""
    timestamp: int
    cpu_usage: float
    memory_usage: float
    gpu_usage: float
    frame_time: float
    active_windows: int
    current_workspace: int

@dataclass
class UserEvent:
    """User interaction event"""
    timestamp: int
    event_type: str
    details: Dict[str, Any]

class PerformanceAnalyzer:
    """Analyzes performance metrics and suggests optimizations"""
    
    def __init__(self):
        self.metrics_history = deque(maxlen=100)
        self.performance_thresholds = {
            'frame_time_warning': 20.0,  # ms (below 50 FPS)
            'frame_time_critical': 33.0,  # ms (below 30 FPS)
            'cpu_warning': 70.0,      # %
            'cpu_critical': 85.0,     # %
            'memory_warning': 75.0,   # %
            'memory_critical': 90.0,  # %
            'gpu_warning': 80.0,      # %
            'gpu_critical': 95.0,     # %
        }
        self.optimization_history = []
    
    def add_metrics(self, metrics: PerformanceMetrics):
        """Add new performance metrics for analysis"""
        self.metrics_history.append(metrics)
        logger.debug(f"Added metrics: CPU {metrics.cpu_usage:.1f}%, "
                    f"Memory {metrics.memory_usage:.1f}%, "
                    f"Frame time {metrics.frame_time:.1f}ms")
    
    def analyze_performance(self) -> Dict[str, Any]:
        """Analyze recent performance data and identify issues"""
        if len(self.metrics_history) < 5:
            return {"status": "insufficient_data"}
        
        recent_metrics = list(self.metrics_history)[-10:]  # Last 10 samples
        
        # Calculate averages
        avg_frame_time = statistics.mean(m.frame_time for m in recent_metrics)
        avg_cpu = statistics.mean(m.cpu_usage for m in recent_metrics)
        avg_memory = statistics.mean(m.memory_usage for m in recent_metrics)
        avg_gpu = statistics.mean(m.gpu_usage for m in recent_metrics)
        
        # Calculate trends
        frame_time_trend = self._calculate_trend([m.frame_time for m in recent_metrics])
        
        analysis = {
            "status": "analyzed",
            "averages": {
                "frame_time": avg_frame_time,
                "cpu_usage": avg_cpu,
                "memory_usage": avg_memory,
                "gpu_usage": avg_gpu,
            },
            "trends": {
                "frame_time_trend": frame_time_trend,  # positive = getting worse
            },
            "issues": [],
            "recommendations": []
        }
        
        # Identify performance issues
        if avg_frame_time > self.performance_thresholds['frame_time_critical']:
            analysis["issues"].append("critical_frame_time")
        elif avg_frame_time > self.performance_thresholds['frame_time_warning']:
            analysis["issues"].append("poor_frame_time")
        
        if avg_cpu > self.performance_thresholds['cpu_critical']:
            analysis["issues"].append("critical_cpu")
        elif avg_cpu > self.performance_thresholds['cpu_warning']:
            analysis["issues"].append("high_cpu")
        
        if avg_memory > self.performance_thresholds['memory_critical']:
            analysis["issues"].append("critical_memory")
        elif avg_memory > self.performance_thresholds['memory_warning']:
            analysis["issues"].append("high_memory")
        
        if avg_gpu > self.performance_thresholds['gpu_critical']:
            analysis["issues"].append("critical_gpu")
        elif avg_gpu > self.performance_thresholds['gpu_warning']:
            analysis["issues"].append("high_gpu")
        
        return analysis
    
    def _calculate_trend(self, values: List[float]) -> float:
        """Calculate trend in values (positive = increasing)"""
        if len(values) < 3:
            return 0.0
        
        # Simple linear regression slope
        n = len(values)
        sum_x = sum(range(n))
        sum_y = sum(values)
        sum_xy = sum(i * values[i] for i in range(n))
        sum_x2 = sum(i * i for i in range(n))
        
        slope = (n * sum_xy - sum_x * sum_y) / (n * sum_x2 - sum_x * sum_x)
        return slope
    
    def generate_optimizations(self, analysis: Dict[str, Any]) -> Dict[str, Any]:
        """Generate AI-driven optimization recommendations"""
        if analysis["status"] != "analyzed":
            return {"changes": {}, "reason": "insufficient_data"}
        
        changes = {}
        reasons = []
        
        issues = analysis["issues"]
        averages = analysis["averages"]
        
        # Frame time optimizations
        if "critical_frame_time" in issues:
            # Aggressive performance mode
            changes.update({
                "effects.blur_radius": 3.0,
                "effects.shadow_size": 10.0,
                "animation_speed": 0.7,
                "effects.antialiasing": 1,
            })
            reasons.append("Critical frame time detected - enabling performance mode")
            
        elif "poor_frame_time" in issues:
            # Moderate performance adjustments
            changes.update({
                "effects.blur_radius": 6.0,
                "animation_speed": 0.85,
                "effects.shadow_size": 15.0,
            })
            reasons.append("Poor frame time detected - reducing visual effects")
        
        # CPU optimizations
        if "critical_cpu" in issues or "high_cpu" in issues:
            changes.update({
                "animation_speed": 0.6,
                "workspace.smooth_scrolling": False,
            })
            reasons.append("High CPU usage - disabling expensive animations")
        
        # GPU optimizations
        if "critical_gpu" in issues:
            changes.update({
                "effects.blur_radius": 2.0,
                "effects.enabled": False,
            })
            reasons.append("Critical GPU usage - disabling visual effects")
            
        elif "high_gpu" in issues:
            changes.update({
                "effects.blur_radius": 4.0,
                "effects.shadow_opacity": 0.4,
            })
            reasons.append("High GPU usage - reducing effect intensity")
        
        # Adaptive optimizations based on window count
        last_metrics = list(self.metrics_history)[-1]
        if last_metrics.active_windows > 8:
            changes.update({
                "animation_speed": 0.8,
                "workspace.animation_duration": 200,
            })
            reasons.append(f"Many windows open ({last_metrics.active_windows}) - optimizing for responsiveness")
        
        # Performance improvement optimizations
        if (not issues and 
            averages["frame_time"] < 12.0 and 
            averages["cpu_usage"] < 40.0 and 
            averages["gpu_usage"] < 50.0):
            # System has headroom - enable more effects
            changes.update({
                "effects.blur_radius": 12.0,
                "effects.shadow_size": 25.0,
                "animation_speed": 1.0,
                "effects.antialiasing": 4,
            })
            reasons.append("System has performance headroom - enabling enhanced visuals")
        
        reason = "; ".join(reasons) if reasons else "No optimizations needed"
        
        # Record optimization for history
        optimization = {
            "timestamp": time.time(),
            "changes": changes,
            "reason": reason,
            "triggered_by": issues,
        }
        self.optimization_history.append(optimization)
        
        return {
            "changes": changes,
            "reason": reason,
            "confidence": self._calculate_confidence(analysis),
        }
    
    def _calculate_confidence(self, analysis: Dict[str, Any]) -> float:
        """Calculate confidence in optimization recommendations"""
        # More samples = higher confidence
        sample_confidence = min(len(self.metrics_history) / 20.0, 1.0)
        
        # Clear issues = higher confidence
        issue_confidence = len(analysis["issues"]) * 0.2
        
        # Trend stability = higher confidence
        trend_confidence = 0.5  # Placeholder
        
        return min(sample_confidence + issue_confidence + trend_confidence, 1.0)

class BehaviorAnalyzer:
    """Analyzes user behavior patterns for predictive optimization"""
    
    def __init__(self):
        self.user_events = deque(maxlen=200)
        self.workspace_usage = defaultdict(int)
        self.window_patterns = defaultdict(int)
        self.time_patterns = defaultdict(list)
    
    def add_user_event(self, event: UserEvent):
        """Record a user interaction event"""
        self.user_events.append(event)
        
        # Update workspace usage statistics
        if event.event_type == "workspace_scroll":
            direction = event.details.get("direction", "unknown")
            self.workspace_usage[f"scroll_{direction}"] += 1
        
        # Track time-based patterns
        hour = time.localtime(event.timestamp).tm_hour
        self.time_patterns[hour].append(event.event_type)
        
        logger.debug(f"Recorded user event: {event.event_type}")
    
    def analyze_patterns(self) -> Dict[str, Any]:
        """Analyze user behavior patterns"""
        if len(self.user_events) < 10:
            return {"status": "insufficient_data"}
        
        recent_events = list(self.user_events)[-20:]
        
        # Analyze workspace scrolling patterns
        scroll_events = [e for e in recent_events if e.event_type == "workspace_scroll"]
        scroll_frequency = len(scroll_events) / max(1, len(recent_events))
        
        # Analyze most common event types
        event_types = [e.event_type for e in recent_events]
        event_frequency = defaultdict(int)
        for event_type in event_types:
            event_frequency[event_type] += 1
        
        most_common_event = max(event_frequency.items(), key=lambda x: x[1]) if event_frequency else ("none", 0)
        
        return {
            "status": "analyzed",
            "scroll_frequency": scroll_frequency,
            "most_common_event": most_common_event[0],
            "event_frequency": dict(event_frequency),
            "workspace_preferences": dict(self.workspace_usage),
        }
    
    def generate_behavioral_optimizations(self, patterns: Dict[str, Any]) -> Dict[str, Any]:
        """Generate optimizations based on user behavior"""
        if patterns["status"] != "analyzed":
            return {"changes": {}, "reason": "insufficient_behavioral_data"}
        
        changes = {}
        reasons = []
        
        # Optimize for frequent workspace scrolling
        if patterns["scroll_frequency"] > 0.3:  # 30% of events are scrolls
            changes.update({
                "workspace.scroll_speed": 1.3,
                "workspace.animation_duration": 180,
                "animation_speed": 1.1,
            })
            reasons.append("Frequent workspace scrolling detected - optimizing for navigation speed")
        
        # Optimize based on most common event
        common_event = patterns["most_common_event"]
        if common_event == "window_focus":
            changes.update({
                "window.focus_animation_speed": 1.2,
                "effects.window_blur": True,
            })
            reasons.append("Frequent window switching - optimizing focus animations")
        
        reason = "; ".join(reasons) if reasons else "No behavioral optimizations suggested"
        
        return {
            "changes": changes,
            "reason": reason,
        }

class LazyUIClient:
    """Main Lazy UI client that connects to Axiom compositor"""
    
    def __init__(self):
        self.socket_path = "/tmp/axiom-lazy-ui.sock"
        self.socket = None
        self.running = False
        
        # AI analyzers
        self.performance_analyzer = PerformanceAnalyzer()
        self.behavior_analyzer = BehaviorAnalyzer()
        
        # State
        self.last_optimization_time = 0
        self.optimization_interval = 5.0  # seconds
        self.compositor_capabilities = []
    
    async def connect(self) -> bool:
        """Connect to the Axiom compositor IPC socket"""
        try:
            self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.socket.connect(self.socket_path)
            logger.info(f"✅ Connected to Axiom IPC: {self.socket_path}")
            return True
        except (ConnectionRefusedError, FileNotFoundError) as e:
            logger.error(f"❌ Failed to connect to Axiom IPC: {e}")
            logger.error("   Make sure Axiom compositor is running")
            return False
    
    def disconnect(self):
        """Disconnect from the compositor"""
        if self.socket:
            self.socket.close()
            self.socket = None
            logger.info("📪 Disconnected from Axiom IPC")
    
    async def send_message(self, message: Dict[str, Any]):
        """Send a message to the compositor"""
        if not self.socket:
            logger.error("❌ Cannot send message - not connected")
            return
        
        try:
            json_msg = json.dumps(message) + "\n"
            self.socket.send(json_msg.encode())
            logger.debug(f"📤 Sent: {json_msg.strip()}")
        except Exception as e:
            logger.error(f"❌ Failed to send message: {e}")
    
    async def receive_message(self) -> Optional[Dict[str, Any]]:
        """Receive a message from the compositor"""
        if not self.socket:
            return None
        
        try:
            # Set socket to non-blocking for async operation
            self.socket.settimeout(0.1)
            data = self.socket.recv(4096).decode().strip()
            
            if data:
                message = json.loads(data)
                logger.debug(f"📨 Received: {data}")
                return message
        except socket.timeout:
            pass  # No data available, continue
        except Exception as e:
            logger.error(f"❌ Failed to receive message: {e}")
        
        return None
    
    async def handle_compositor_message(self, message: Dict[str, Any]):
        """Process incoming messages from the compositor"""
        msg_type = message.get("type")
        
        if msg_type == "StartupComplete":
            self.compositor_capabilities = message.get("capabilities", [])
            logger.info(f"🚀 Compositor started with capabilities: {', '.join(self.compositor_capabilities)}")
        
        elif msg_type == "PerformanceMetrics":
            metrics = PerformanceMetrics(
                timestamp=message["timestamp"],
                cpu_usage=message["cpu_usage"],
                memory_usage=message["memory_usage"],
                gpu_usage=message["gpu_usage"],
                frame_time=message["frame_time"],
                active_windows=message["active_windows"],
                current_workspace=message["current_workspace"],
            )
            self.performance_analyzer.add_metrics(metrics)
            logger.debug(f"📊 Performance metrics: {metrics.frame_time:.1f}ms frame time")
        
        elif msg_type == "UserEvent":
            event = UserEvent(
                timestamp=message["timestamp"],
                event_type=message["event_type"],
                details=message["details"],
            )
            self.behavior_analyzer.add_user_event(event)
            logger.debug(f"👤 User event: {event.event_type}")
        
        elif msg_type == "ConfigResponse":
            logger.info(f"⚙️ Config response: {message['key']} = {message['value']}")
    
    async def run_optimization_cycle(self):
        """Run AI optimization analysis and send recommendations"""
        current_time = time.time()
        
        if current_time - self.last_optimization_time < self.optimization_interval:
            return
        
        self.last_optimization_time = current_time
        
        logger.info("🧠 Running AI optimization analysis...")
        
        # Analyze current performance
        perf_analysis = self.performance_analyzer.analyze_performance()
        behavior_patterns = self.behavior_analyzer.analyze_patterns()
        
        if perf_analysis["status"] == "analyzed":
            # Generate performance optimizations
            perf_optimizations = self.performance_analyzer.generate_optimizations(perf_analysis)
            
            if perf_optimizations["changes"]:
                logger.info(f"🎯 Performance optimization: {perf_optimizations['reason']}")
                
                # Send optimization to compositor
                await self.send_message({
                    "type": "OptimizeConfig",
                    "changes": perf_optimizations["changes"],
                    "reason": perf_optimizations["reason"],
                })
        
        if behavior_patterns["status"] == "analyzed":
            # Generate behavioral optimizations  
            behavior_optimizations = self.behavior_analyzer.generate_behavioral_optimizations(behavior_patterns)
            
            if behavior_optimizations["changes"]:
                logger.info(f"👤 Behavioral optimization: {behavior_optimizations['reason']}")
                
                # Send optimization to compositor
                await self.send_message({
                    "type": "OptimizeConfig",
                    "changes": behavior_optimizations["changes"],
                    "reason": behavior_optimizations["reason"],
                })
    
    async def health_check_cycle(self):
        """Periodically request health check from compositor"""
        await asyncio.sleep(10)  # Wait 10 seconds between health checks
        
        while self.running:
            await self.send_message({"type": "HealthCheck"})
            await asyncio.sleep(30)  # Health check every 30 seconds
    
    async def run(self):
        """Main event loop for the Lazy UI client"""
        logger.info("🧠 Starting Lazy UI - AI Compositor Optimization System")
        
        if not await self.connect():
            return 1
        
        self.running = True
        
        # Set up signal handlers
        def signal_handler(signum, frame):
            logger.info("📨 Received shutdown signal")
            self.running = False
        
        signal.signal(signal.SIGINT, signal_handler)
        signal.signal(signal.SIGTERM, signal_handler)
        
        # Start health check task
        health_task = asyncio.create_task(self.health_check_cycle())
        
        logger.info("✨ Lazy UI is ready! AI optimization system active.")
        
        try:
            while self.running:
                # Process incoming messages
                message = await self.receive_message()
                if message:
                    await self.handle_compositor_message(message)
                
                # Run optimization cycle
                await self.run_optimization_cycle()
                
                # Small delay to prevent busy loop
                await asyncio.sleep(0.1)
                
        except KeyboardInterrupt:
            logger.info("📨 Keyboard interrupt received")
        finally:
            health_task.cancel()
            self.disconnect()
            logger.info("👋 Lazy UI shutdown complete")
        
        return 0

async def main():
    """Main entry point"""
    logger.info("🚀 Lazy UI - AI-Powered Compositor Optimization")
    logger.info("=" * 50)
    
    client = LazyUIClient()
    return await client.run()

if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
