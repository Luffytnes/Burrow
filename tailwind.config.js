/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        bg: {
          deep:  "#0f0f0f",
          base:  "#181818",
          card:  "#212121",
          hover: "#272727",
        },
        accent: {
          DEFAULT: "#ff0000",
          light:   "#ff4444",
          bright:  "#ff6b6b",
        },
        // Remap green → YouTube palette so all green-* classes use YT colors
        green: {
          300: "#aaaaaa",  // secondary / muted text
          400: "#ff0000",  // YouTube red (primary accent)
          500: "#cc0000",  // darker red
          600: "#991b1b",  // deep red
        },
      },
      fontFamily: {
        sans: ["-apple-system", "BlinkMacSystemFont", "SF Pro Display", "Helvetica Neue", "sans-serif"],
      },
      boxShadow: {
        glow:    "0 0 40px rgba(255,0,0,0.25)",
        "glow-sm": "0 0 20px rgba(255,0,0,0.15)",
        card:    "0 8px 32px rgba(0,0,0,0.6)",
      },
    },
  },
  plugins: [],
};
