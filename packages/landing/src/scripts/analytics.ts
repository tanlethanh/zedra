import { initializeApp } from "firebase/app";
import { getAnalytics, isSupported, logEvent } from "firebase/analytics";

const firebaseConfig = {
  apiKey: import.meta.env.PUBLIC_FIREBASE_API_KEY,
  authDomain: import.meta.env.PUBLIC_FIREBASE_AUTH_DOMAIN,
  projectId: import.meta.env.PUBLIC_FIREBASE_PROJECT_ID,
  storageBucket: import.meta.env.PUBLIC_FIREBASE_STORAGE_BUCKET,
  messagingSenderId: import.meta.env.PUBLIC_FIREBASE_MESSAGING_SENDER_ID,
  appId: import.meta.env.PUBLIC_FIREBASE_APP_ID,
  measurementId: import.meta.env.PUBLIC_FIREBASE_MEASUREMENT_ID,
};

function hasFirebaseConfig() {
  return Object.values(firebaseConfig).every(
    (value) => typeof value === "string" && value.length > 0,
  );
}

async function setupAnalytics() {
  if (!hasFirebaseConfig()) {
    return;
  }

  if (!(await isSupported())) {
    return;
  }

  const app = initializeApp(firebaseConfig);
  const analytics = getAnalytics(app);

  const track = (
    eventName: string,
    params: Record<string, string | number | boolean>,
  ) => {
    logEvent(analytics, eventName, params);
  };

  for (const input of document.querySelectorAll<HTMLInputElement>(
    "[data-analytics-install-tab]",
  )) {
    input.addEventListener("change", () => {
      if (!input.checked) {
        return;
      }

      track("install_method_selected", {
        method: input.dataset.analyticsInstallTab ?? "unknown",
      });
    });
  }

  for (const link of document.querySelectorAll<HTMLAnchorElement>(
    "[data-analytics-store]",
  )) {
    link.addEventListener("click", () => {
      track("store_click", {
        platform: link.dataset.analyticsStore ?? "unknown",
      });
    });
  }

  for (const link of document.querySelectorAll<HTMLAnchorElement>(
    "[data-analytics-social]",
  )) {
    link.addEventListener("click", () => {
      track("social_click", {
        network: link.dataset.analyticsSocial ?? "unknown",
      });
    });
  }
}

void setupAnalytics();
