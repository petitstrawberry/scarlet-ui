const status = document.querySelector('#gpu-status');
const button = document.querySelector('#check-gpu');

function unavailableReason() {
  if (!globalThis.isSecureContext) return 'This page is not a secure context. Use HTTPS or localhost to access WebGPU.';
  if (!navigator.gpu) return 'WebGPU is not exposed by this browser. Support may depend on the browser, device or administrator settings.';
  return null;
}

const reason = unavailableReason();
if (reason) status.textContent = reason;

button.addEventListener('click', async () => {
  button.disabled = true;
  status.textContent = 'Checking for a WebGPU adapter…';
  try {
    const reason = unavailableReason();
    if (reason) { status.textContent = reason; return; }
    // Do not request a device, start a sample or query identifying GPU information.
    const adapter = await navigator.gpu.requestAdapter();
    status.textContent = adapter
      ? 'A WebGPU adapter is available. Open a sample to initialize its renderer.'
      : 'WebGPU is exposed, but no adapter is available. Your device, browser settings or graphics driver may not support this configuration.';
  } catch (error) {
    status.textContent = `The WebGPU check could not complete: ${error instanceof Error ? error.message : String(error)}`;
  } finally {
    button.disabled = false;
  }
});
