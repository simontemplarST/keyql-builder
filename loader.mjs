// Trunk initializer: shows download progress on the loading screen in index.html.
// The loading screen itself is removed from Rust (src/main.rs) once the app has drawn its first frame.

const $ = (id) => document.getElementById(id);

function mb(bytes) {
  return (bytes / 1048576).toFixed(1) + " MB";
}

export default function loader() {
  return {
    onStart() {
      $("status").textContent = "Downloading the app…";
    },
    onProgress({ current, total }) {
      // Compressed responses carry no length, so the bar can only fill when the server sends one.
      if (total > 0) {
        $("bar").classList.remove("indeterminate");
        $("fill").style.width = Math.min(100, (current / total) * 100).toFixed(1) + "%";
        $("status").textContent = `Downloading the app… ${mb(current)} of ${mb(total)}`;
      } else {
        $("status").textContent = `Downloading the app… ${mb(current)}`;
      }
    },
    onComplete() {
      $("bar").classList.add("indeterminate");
      $("status").textContent = "Starting up…";
    },
    onSuccess() {},
    onFailure(error) {
      $("bar").style.display = "none";
      $("status").style.display = "none";
      $("failed").style.display = "block";
      $("failed-why").textContent = "Please use a current version of Edge, Chrome, Firefox or Safari.";
      console.error(error);
    },
  };
}
