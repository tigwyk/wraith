# Per-profile appearance: acceptance

## Probe

Throwaway example (deleted): a 400x300 popup shown with `SW_SHOWNOACTIVATE` over a solid #FF0000 helper window, cleared each frame to premultiplied `[0,0,0,0.5]`. Pixel read from the screen centre with `Graphics.CopyFromScreen`, no window activated.

| Mode | Swap chain | Centre pixel | Median | p99 |
| --- | --- | --- | --- | --- |
| Composition | `CreateSwapChainForComposition`, `ALPHA_MODE_PREMULTIPLIED`, `WS_EX_NOREDIRECTIONBITMAP`, DirectComposition target/visual | #800000 | 0.043 ms | 0.309 ms |
| HWND (today) | `CreateSwapChainForHwnd`, `ALPHA_MODE_IGNORE` | #000000 | 0.081 ms | 0.293 ms |

Latency is `WaitForSingleObjectEx` on the frame-latency waitable returning to `Present(1)` returning, over 600 frames per mode, unoptimised (dev profile) build. Both modes: B8G8R8A8, flip-discard, `SetMaximumFrameLatency(1)`.

Machine: Windows 11 Enterprise 10.0.26300, NVIDIA GeForce RTX 5070 Ti.

Verdict: a composition swap chain with premultiplied alpha blends with what is behind the window (red at 50% black gives #800000), so spec section 4 holds. Composition p99 is 0.016 ms above HWND p99, well under the 1 ms threshold. The opt-in rule (Task 7) stands.
