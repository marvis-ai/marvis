import Counter from "@/components/Counter";

const HomePage = () => {
  return (
    <div className="flex flex-col items-center justify-center h-screen gap-4">
      <h1 className="text-3xl font-bold">Hello Home Page</h1>
      <p className="text-lg text-muted-foreground">
        Welcome to the Marvis AI web application.
      </p>
      <Counter />
    </div>
  );
};

export default HomePage;
