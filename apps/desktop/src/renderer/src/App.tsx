import { useEffect, useState } from "react";
import { Onboarding } from "@renderer/components/Onboarding";

const App = (): React.JSX.Element => {
  const [isOnboarding, setIsOnboarding] = useState<boolean>(false);

  useEffect(() => {
    const timer = setTimeout(() => {
      setIsOnboarding(true);
    }, 4000);

    return () => {
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    if (isOnboarding) {
      console.log(window.electron);
    }
  }, [isOnboarding]);

  return (
    <main className="flex w-full h-full flex-col items-center justify-center relative px-6 py-2 draggable">
      {isOnboarding ? (
        <Onboarding />
      ) : (
        <h1 className="text-6xl font-bold text-center text-white/90">
          Marvis AI
        </h1>
      )}
    </main>
  );
};

export default App;
