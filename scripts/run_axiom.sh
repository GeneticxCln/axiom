#!/bin/bash
# Run Axiom Compositor with On-Screen Rendering

echo "🚀 Starting Axiom Compositor"
echo ""
echo "This will open a window showing the Axiom compositor."
echo "You can then launch Wayland applications in it."
echo ""

# Run the presenter (it will fullscreen by default)
exec ./target/release/run_present_winit --backend auto